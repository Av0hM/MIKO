//! Validated, edge-triggered automation with hot reload and a bounded action queue.
use crate::{modules::Module, state::{ConditionOperator, Rule, RuleAction, RuleCondition, State}};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc::{SyncSender, sync_channel}};
use std::time::Duration;

pub struct AutomationModule {
    rules: Mutex<Vec<Rule>>,
    matched: HashMap<String,bool>,
    path: PathBuf,
    source: Option<Vec<u8>>,
    sender: Option<SyncSender<RuleAction>>,
    stop: Arc<AtomicBool>,
}
impl AutomationModule {
    /// Construct an automation engine. Rules are loaded during initialization.
    pub fn new()->Result<Self>{Self::at(PathBuf::from(std::env::var("HOME")?).join(".config/samos/rules.json"))}
    fn at(path:PathBuf)->Result<Self>{Ok(Self{rules:Mutex::new(Vec::new()),matched:HashMap::new(),path,source:None,sender:None,stop:Arc::new(AtomicBool::new(false))})}
    fn reload(&mut self)->Result<()> {
        let bytes=match std::fs::read(&self.path){Ok(bytes)=>bytes,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>b"[]".to_vec(),Err(e)=>return Err(e.into())};
        if self.source.as_ref()==Some(&bytes){return Ok(());}
        self.source=Some(bytes.clone());
        let rules=Self::validate_rules(serde_json::from_slice(&bytes)?)?;
        let ids:HashSet<_>=rules.iter().map(|r|r.id.clone()).collect();
        self.matched.retain(|id,_|ids.contains(id));
        *self.rules.lock().map_err(|_|anyhow::anyhow!("Rules unavailable"))?=rules;
        Ok(())
    }
    /// Validate the complete ruleset, including field types and action arguments.
    pub fn validate_rules(mut rules:Vec<Rule>)->Result<Vec<Rule>> {
        ensure!(rules.len()<=128,"At most 128 automation rules are supported");
        let sample=serde_json::to_value(State::default())?;
        let mut ids=HashSet::new();
        for rule in &mut rules {
            if rule.id.is_empty(){rule.id=uuid::Uuid::new_v4().to_string();}
            ensure!(ids.insert(rule.id.clone()),"Duplicate rule ID: {}",rule.id);
            ensure!(!rule.name.trim().is_empty(),"Rule name cannot be empty");
            ensure!(!rule.conditions.is_empty() && rule.conditions.len()<=16,"Rules need 1–16 conditions");
            ensure!(!rule.actions.is_empty() && rule.actions.len()<=8,"Rules need 1–8 actions");
            for condition in &rule.conditions {
                let value=field(&sample,&condition.field).context(format!("Unknown rule field: {}",condition.field))?;
                match condition.operator {
                    ConditionOperator::GreaterThan|ConditionOperator::LessThan=>ensure!(value.is_number()&&condition.value.is_number(),"Numeric comparison needs numeric values"),
                    ConditionOperator::Contains|ConditionOperator::StartsWith|ConditionOperator::EndsWith=>ensure!(value.is_string()&&condition.value.is_string(),"String comparison needs string values"),
                    _=>ensure!((value.is_number()&&condition.value.is_number())||(value.is_string()&&condition.value.is_string())||(value.is_boolean()&&condition.value.is_boolean()),"Condition value has the wrong type"),
                }
            }
            for action in &rule.actions {validate_action(action)?;}
        }
        Ok(rules)
    }
    fn save(&self,rules:&[Rule])->Result<()> {
        let parent=self.path.parent().context("Rules path needs a parent")?;
        std::fs::create_dir_all(parent)?;
        let temp=parent.join(format!(".rules-{}.tmp",uuid::Uuid::new_v4()));
        std::fs::write(&temp,serde_json::to_vec_pretty(rules)?)?;
        if let Err(error)=std::fs::rename(&temp,&self.path){let _=std::fs::remove_file(&temp);return Err(error.into());}
        Ok(())
    }
    /// Add and persist a validated rule without losing the old rules on write failure.
    pub fn add_rule(&self,rule:Rule)->Result<()> {
        let mut current=self.rules.lock().map_err(|_|anyhow::anyhow!("Rules unavailable"))?;
        let mut candidate=current.clone();candidate.push(rule);
        let candidate=Self::validate_rules(candidate)?;self.save(&candidate)?;*current=candidate;Ok(())
    }
    /// Remove a rule and persist the result.
    pub fn remove_rule(&self,id:&str)->Result<()> {
        let mut current=self.rules.lock().map_err(|_|anyhow::anyhow!("Rules unavailable"))?;
        let candidate:Vec<_>=current.iter().filter(|r|r.id!=id).cloned().collect();
        self.save(&candidate)?;*current=candidate;Ok(())
    }
    /// Return a snapshot of loaded rules.
    pub fn get_rules(&self)->Vec<Rule>{self.rules.lock().map(|r|r.clone()).unwrap_or_default()}
    fn triggered(&mut self,rule:&Rule,state:&Value)->bool {
        let matches=rule.enabled&&rule.conditions.iter().all(|c|evaluate(c,state));
        let previous=self.matched.insert(rule.id.clone(),matches).unwrap_or(false);
        matches&&!previous
    }
}
fn field<'a>(state:&'a Value,path:&str)->Option<&'a Value>{
    let parts:Vec<_>=path.split('.').collect();
    if parts.len()>2 || parts.is_empty(){return None;}
    let mut current=state;
    for part in parts {current=current.get(part)?;}
    if current.is_array()||current.is_object()||current.is_null(){None}else{Some(current)}
}
fn evaluate(condition:&RuleCondition,state:&Value)->bool {
    let Some(a)=field(state,&condition.field) else{return false;};let b=&condition.value;
    match condition.operator {
        ConditionOperator::Equals=>if a.is_number()&&b.is_number(){a.as_f64()==b.as_f64()}else{a==b},
        ConditionOperator::NotEquals=>if a.is_number()&&b.is_number(){a.as_f64()!=b.as_f64()}else{a!=b},
        ConditionOperator::GreaterThan=>a.as_f64().zip(b.as_f64()).is_some_and(|(a,b)|a>b),
        ConditionOperator::LessThan=>a.as_f64().zip(b.as_f64()).is_some_and(|(a,b)|a<b),
        ConditionOperator::Contains=>a.as_str().zip(b.as_str()).is_some_and(|(a,b)|a.contains(b)),
        ConditionOperator::StartsWith=>a.as_str().zip(b.as_str()).is_some_and(|(a,b)|a.starts_with(b)),
        ConditionOperator::EndsWith=>a.as_str().zip(b.as_str()).is_some_and(|(a,b)|a.ends_with(b)),
    }
}
fn app_id(id:&str)->bool{!id.is_empty()&&!id.starts_with('-')&&id.len()<=200&&id.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))}
fn command_parts(command:&str)->Result<Vec<&str>>{
    ensure!(!command.chars().any(|c|";|&`$<>\\\n\r\"'".contains(c)),"Shell syntax is not allowed; use a structured action for text with spaces or punctuation");
    let parts:Vec<_>=command.split_whitespace().collect();
    match parts.as_slice(){
        ["samosctl", "wifi-toggle"|"bluetooth-toggle"]=>{},
        ["samosctl","theme-set",name]=>crate::config::validate_name(name)?,
        ["samosctl","power-profile",profile]|["powerprofilesctl","set",profile]=>ensure!(["balanced","power-saver","performance"].contains(profile),"Invalid power profile"),
        ["hyprctl","dispatch","workspace",id]=>ensure!(id.parse::<u32>().is_ok_and(|n|(1..=1000).contains(&n)),"Invalid workspace"),
        ["gtk-launch",id]=>ensure!(app_id(id),"Invalid application ID"),
        ["notify-send",rest @ ..]=>ensure!(!rest.is_empty() && !rest.iter().any(|s|s.starts_with('-')),"Use plain notification text"),
        _=>bail!("Command is not on the automation allowlist"),
    }
    Ok(parts)
}
fn validate_action(action:&RuleAction)->Result<()> {
    match action {
        RuleAction::RunCommand{command}=>{command_parts(command)?;},
        RuleAction::SetTheme{theme}=>crate::config::validate_name(theme)?,
        RuleAction::SetPowerProfile{profile}=>ensure!(["balanced","power-saver","performance"].contains(&profile.as_str()),"Invalid power profile"),
        RuleAction::SwitchWorkspace{workspace_id}=>ensure!((1..=1000).contains(workspace_id),"Invalid workspace"),
        RuleAction::LaunchApp{app_id:id}=>ensure!(app_id(id),"Invalid app ID"),
        RuleAction::Speak{text}=>ensure!(!text.trim().is_empty()&&text.len()<=4000,"Speech must contain 1–4000 bytes"),
        _=>{},
    }Ok(())
}
fn execute(action:&RuleAction)->Result<()> {
    use crate::process::run;
    match action {
        RuleAction::Notify=>{run("notify-send",&["SamOS","Automation rule triggered"],None,5)?;},
        RuleAction::Log{message}=>eprintln!("[automation] {message}"),
        RuleAction::RunCommand{command}=>{let parts=command_parts(command)?;run(parts[0],&parts[1..],None,8)?;},
        RuleAction::SetTheme{theme}=>{let path=crate::config::Config::path()?;ensure!(path.parent().context("Config directory missing")?.join("themes").join(format!("{theme}.toml")).is_file(),"Theme not found");crate::config::set_theme_at(&path,theme)?;},
        RuleAction::SetPowerProfile{profile}=>{run("powerprofilesctl",&["set",profile],None,8)?;},
        RuleAction::SwitchWorkspace{workspace_id}=>{run("hyprctl",&["dispatch","workspace",&workspace_id.to_string()],None,5)?;},
        RuleAction::LaunchApp{app_id}=>{run("gtk-launch",&[app_id],None,8)?;},
        RuleAction::ToggleWifi|RuleAction::ToggleBluetooth=>{
            let binary=PathBuf::from(std::env::var("HOME")?).join(".local/bin/samosctl");
            run(&binary.to_string_lossy(),&[if matches!(action,RuleAction::ToggleWifi){"wifi-toggle"}else{"bluetooth-toggle"}],None,10)?;
        },
        RuleAction::Speak{text}=>{
            let home=PathBuf::from(std::env::var("HOME")?);
            let directory=std::env::temp_dir().join(format!("samos-automation-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&directory)?;
            let result=(||->Result<()>{
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&directory,std::fs::Permissions::from_mode(0o700))?;
                let output=directory.join("speech.wav");let model=home.join(".local/share/piper/voices/en_US-lessac-medium.onnx");
                let data=format!("ESPEAK_DATA_PATH={}",home.join(".local/share/espeak-ng-data").display());let libraries=format!("LD_LIBRARY_PATH={}",home.join(".local/lib").display());
                run("env",&[&data,&libraries,&home.join(".local/bin/piper").to_string_lossy(),"--model",&model.to_string_lossy(),"--output_file",&output.to_string_lossy()],Some(text),60)?;
                run("aplay",&["-q",&output.to_string_lossy()],None,90)?;Ok(())
            })();let _=std::fs::remove_dir_all(directory);result?;
        }
    }Ok(())
}
impl Module for AutomationModule {
    fn name(&self)->&'static str{"automation"}
    fn init(&mut self)->Result<()> {
        if let Err(e)=self.reload(){eprintln!("[automation] No valid rules loaded: {e}");}
        let (sender,receiver)=sync_channel::<RuleAction>(16);self.sender=Some(sender);let stop=self.stop.clone();
        std::thread::spawn(move ||{
            while !stop.load(Ordering::Relaxed){
                match receiver.recv_timeout(Duration::from_millis(200)){
                    Ok(action)=>if let Err(e)=execute(&action){eprintln!("[automation] Action failed: {e}");},
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)=>{},Err(_)=>break,
                }
            }
        });Ok(())
    }
    fn update(&mut self,state:&mut State)->Result<()> {
        if let Err(e)=self.reload(){eprintln!("[automation] Keeping last valid rules: {e}");}
        let rules=self.get_rules();state.automation.enabled=true;state.automation.rules_count=rules.len();
        let value=serde_json::to_value(&state)?;
        for rule in rules {if self.triggered(&rule,&value){for action in rule.actions {if let Some(sender)=&self.sender {if let Err(error)=sender.try_send(action){eprintln!("[automation] Action queue full or unavailable: {error}");}}}}}
        Ok(())
    }
    fn shutdown(&mut self)->Result<()>{self.stop.store(true,Ordering::Relaxed);self.sender.take();Ok(())}
}
#[cfg(test)]
mod tests {
    use super::*;use serde_json::json;
    fn rule()->Rule{Rule{id:"battery".into(),name:"Low battery".into(),enabled:true,conditions:vec![RuleCondition{field:"battery.percent".into(),operator:ConditionOperator::LessThan,value:json!(20)}],actions:vec![RuleAction::Log{message:"low".into()}]}}
    #[test]fn reject_shell_bypass(){for command in ["notify-send ok; printf bad","notify-send ok\nprintf bad","gtk-launch $(id)","samosctl unexpected"]{assert!(command_parts(command).is_err());}assert!(command_parts("hyprctl dispatch workspace 2").is_ok());}
    #[test]fn reject_invalid_fields_and_duplicates(){let mut invalid=rule();invalid.conditions[0].field="cpu.typo".into();assert!(AutomationModule::validate_rules(vec![invalid]).is_err());assert!(AutomationModule::validate_rules(vec![rule(),rule()]).is_err());}
    #[test]fn trigger_only_on_transition(){let mut module=AutomationModule::at(PathBuf::from("/tmp/not-read")).unwrap();let mut state=json!({"battery":{"percent":10},"cpu":{"usage":1}});assert!(module.triggered(&rule(),&state));state["cpu"]["usage"]=json!(99);assert!(!module.triggered(&rule(),&state));state["battery"]["percent"]=json!(50);assert!(!module.triggered(&rule(),&state));state["battery"]["percent"]=json!(10);assert!(module.triggered(&rule(),&state));}
    #[test]fn reload_keeps_valid_rules_on_error_and_clears_on_delete(){let path=std::env::temp_dir().join(format!("samos-rules-{}.json",uuid::Uuid::new_v4()));std::fs::write(&path,serde_json::to_vec(&vec![rule()]).unwrap()).unwrap();let mut module=AutomationModule::at(path.clone()).unwrap();module.reload().unwrap();assert_eq!(module.get_rules().len(),1);std::fs::write(&path,b"invalid").unwrap();assert!(module.reload().is_err());assert_eq!(module.get_rules().len(),1);std::fs::remove_file(path).unwrap();module.reload().unwrap();assert!(module.get_rules().is_empty());}
}
