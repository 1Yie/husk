//! Settings operations run on the host owner, never on the UI thread.
use agent_llm::AppConfig;
use husk_host::session_manager::SessionManager;
use serde_json::{Value, json};
use std::path::PathBuf;

const MASK: &str = "••••••••";

#[derive(Debug, Clone)]
pub enum SettingsCommand {
    Refresh,
    Terminal(Value),
    ForkSession(super::SessionKey),
    SetSessionArchived {
        key: super::SessionKey,
        archived: bool,
    },
    ExportSession {
        key: super::SessionKey,
        path: PathBuf,
    },
    RemoveWorkspace(String),
    Preferences(Value),
    Appearance(Value),
    SaveProvider {
        original: Option<String>,
        id: String,
        value: Value,
    },
    DeleteProvider(String),
    SaveModel {
        provider: String,
        original: Option<String>,
        value: Value,
    },
    DeleteModel {
        provider: String,
        id: String,
    },
    Instructions {
        scope: String,
        content: String,
    },
    SaveRule(Value),
    DeleteRule {
        name: String,
        global: bool,
    },
    SaveSubagent(Value),
    DeleteSubagent {
        name: String,
        global: bool,
    },
    SaveSkill {
        name: String,
        global: bool,
        content: String,
    },
    DeleteSkill {
        name: String,
        global: bool,
    },
    SaveMcp {
        id: String,
        value: Value,
    },
    DeleteMcp(String),
    MemorySet {
        scope: String,
        key: String,
        value: String,
    },
    MemoryForget {
        scope: String,
        key: String,
    },
    MemoryRemoveFact(u64),
    MemoryClear(String),
    SemanticRetry,
    SemanticCancel,
    SemanticRemove,
    FallbackChain(Vec<String>),
    PluginEnabled {
        id: String,
        enabled: bool,
    },
    PluginTrust(String),
    PluginRemove(String),
    PluginReload,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

fn safe_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name.contains("..")
        || name
            .chars()
            .any(|c| !c.is_alphanumeric() && !matches!(c, '-' | '_' | '.'))
    {
        Err("名称只能包含字母、数字、点、横线和下划线".into())
    } else {
        Ok(())
    }
}

fn mask_secrets(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "api_key" | "apiKey" | "headers" | "env") {
                    if !value.is_null()
                        && value.as_str() != Some("")
                        && value.as_object().is_none_or(|v| !v.is_empty())
                    {
                        *value = json!(MASK);
                    }
                } else {
                    mask_secrets(value);
                }
            }
        }
        Value::Array(items) => {
            for value in items {
                mask_secrets(value);
            }
        }
        _ => {}
    }
}

fn restore_secrets(value: &mut Value, stored: &Value) {
    if value.as_str() == Some(MASK) {
        *value = stored.clone();
        return;
    }
    if let Value::Object(object) = value {
        for (key, value) in object {
            restore_secrets(value, &stored[key]);
        }
    } else if let Value::Array(items) = value {
        for (index, value) in items.iter_mut().enumerate() {
            restore_secrets(value, &stored[index]);
        }
    }
}

fn memory(manager: &SessionManager) -> Result<agent_context::memory::MemoryStore, String> {
    let dir = agent_store::app_data_dir().ok_or("记忆数据目录不可用")?;
    agent_context::memory::MemoryStore::open(&dir.join("memory.db"), &manager.workspace_root)
}

fn scope(value: &str) -> Result<agent_context::memory::PersonaScope, String> {
    match value {
        "global" => Ok(agent_context::memory::PersonaScope::Global),
        "workspace" => Ok(agent_context::memory::PersonaScope::Workspace),
        _ => Err("无效的记忆作用域".into()),
    }
}

fn skill_dir(manager: &SessionManager, global: bool) -> Result<PathBuf, String> {
    if global {
        Ok(dirs::home_dir()
            .ok_or("用户目录不可用")?
            .join(".pi/agent/skills"))
    } else {
        Ok(manager.workspace_root.join(".pi/skills"))
    }
}

fn skills(manager: &SessionManager) -> Vec<Value> {
    let mut rows = Vec::new();
    for global in [true, false] {
        let Ok(dir) = skill_dir(manager, global) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            if let Ok(content) = std::fs::read_to_string(&path) {
                rows.push(json!({"name": entry.file_name().to_string_lossy(), "path": path, "global":global, "content":content, "writable": true}));
            }
        }
    }
    rows.sort_by_key(|v| text(v, "name").to_owned());
    rows
}

fn mcps(paths: &agent_paths::AppPaths) -> Vec<Value> {
    let mut rows = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.config_dir.join("mcp")) {
        for entry in entries.flatten() {
            let path = entry.path().join("manifest.json");
            let id = entry.file_name().to_string_lossy().into_owned();
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()))
            {
                Ok(mut value) => {
                    mask_secrets(&mut value);
                    rows.push(json!({"id":id,"manifest":value,"path":path}));
                }
                Err(error) => rows.push(json!({"id":id,"error":error,"path":path})),
            }
        }
    }
    rows.sort_by_key(|v| text(v, "id").to_owned());
    rows
}

fn load_preferences() -> Result<husk_host::prefs::DefaultPreferences, String> {
    match agent_store::try_global_get::<husk_host::prefs::DefaultPreferences>("default_preferences")
    {
        Ok(None) => Ok(husk_host::prefs::DefaultPreferences::default()),
        Ok(Some(value)) => Ok(husk_host::prefs::try_load_default_preferences().unwrap_or(value)),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn snapshot(manager: &SessionManager, paths: &agent_paths::AppPaths) -> Value {
    let archives = agent_store::SessionStore::open_existing(&manager.workspace_root)
        .map(|store| {
            store
                .list()
                .into_iter()
                .filter(|row| row.archived)
                .map(|row| {
                    json!({
                        "root":manager.workspace_root,
                        "id":row.id,
                        "title":row.title,
                        "preview":row.preview,
                        "pinned":row.pinned,
                        "pinned_global":row.pinned_global,
                        "updated_at":row.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let config = match AppConfig::load(&paths.config_dir, None) {
        Ok(config) => {
            let mut value = serde_json::to_value(config).unwrap_or_default();
            mask_secrets(&mut value);
            json!({"value":value})
        }
        Err(error) => json!({"error":error.to_string()}),
    };
    let (preferences, preferences_error) = match load_preferences() {
        Ok(value) => (serde_json::to_value(value).unwrap_or_default(), None),
        Err(error) => (Value::Null, Some(error)),
    };
    let rules = husk_host::rules::discover_rules(&manager.workspace_root, paths).into_iter().map(|r|json!({"name":r.name,"description":r.description,"globs":r.globs,"always":r.always,"body":r.body,"global":r.global,"path":r.path})).collect::<Vec<_>>();
    let agents = husk_host::subagents::agent_rows(&manager.workspace_root, paths).into_iter().map(|a|json!({"name":a.name,"description":a.description,"prompt":a.prompt,"model":a.model,"thinking":a.thinking,"readonly":a.readonly,"global":a.global,"path":a.path,"builtin":a.path.is_empty()})).collect::<Vec<_>>();
    // A missing store is not an empty store. Preserve every read error for the UI.
    let memory = memory(manager).and_then(|store|Ok(json!({"persona":store.all_persona()?,"facts":store.list_facts(200)?.into_iter().map(|f|json!({"id":f.id,"text":f.text,"confidence":f.confidence,"created_at":f.created_at})).collect::<Vec<_>>(),"episodes":store.count_episodes()?}))).unwrap_or_else(|error|json!({"error":error}));
    let terminal = crate::features::terminal::TerminalPreferences::load(&paths.config_dir);
    let (terminal, terminal_error) = match terminal {
        Ok(value) => (json!(value), None),
        Err(error) => (Value::Null, Some(error)),
    };
    let mut output = json!({
        "workspace": manager.workspace_root,
        "archives": archives,
        "config":config,
        "preferences":preferences,
        "preferencesError":preferences_error,
        "appearance":husk_host::prefs::load_appearance_settings(),
        "terminal":terminal, "terminalError":terminal_error,
        "instructions":manager.instructions(),
        "rules":rules, "subagents":agents, "skills":skills(manager),
        "mcp":mcps(paths), "plugins":manager.plugin_overview(),
        "memory":memory, "stats":manager.usage_stats(),
        "config_path":AppConfig::default_path(&paths.config_dir),
        "semantic":{"compiled":agent_context::memory::semantic::available(),"ready":agent_context::memory::semantic::is_ready(),"cached":agent_context::memory::semantic::model_cached(), "name":agent_context::memory::semantic::model_name(), "progress":agent_context::memory::semantic::progress()},
    });
    mask_secrets(&mut output);
    output
}

fn save_config(
    value: Value,
    manager: &mut SessionManager,
    paths: &agent_paths::AppPaths,
) -> Result<(), String> {
    let config: AppConfig = serde_json::from_value(value).map_err(|e| format!("配置无效：{e}"))?;
    for (id, provider) in &config.providers {
        if id.trim().is_empty() || provider.base_url.trim().is_empty() {
            return Err("提供商 ID 与 API 地址不能为空".into());
        }
        let mut seen = std::collections::HashSet::new();
        for model in &provider.models {
            if model.id().trim().is_empty() || !seen.insert(model.id()) {
                return Err("模型 ID 不能为空或重复".into());
            }
        }
    }
    AppConfig::save_to(&AppConfig::default_path(&paths.config_dir), &config)
        .map_err(|e| e.to_string())?;
    let (notified, live) = manager.apply_loaded_config(config);
    if notified != live {
        return Err(format!(
            "已写入磁盘，但只有 {notified}/{live} 个会话接受热重载；请重试"
        ));
    }
    Ok(())
}

fn edit_config(
    manager: &mut SessionManager,
    paths: &agent_paths::AppPaths,
    edit: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    // Never overwrite an unreadable config with defaults.
    let config = AppConfig::load(&paths.config_dir, None).map_err(|e| e.to_string())?;
    let mut value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    edit(&mut value)?;
    save_config(value, manager, paths)
}

fn patch_preferences(patch: Value, manager: &mut SessionManager) -> Result<(), String> {
    let stored = load_preferences()?;
    let mut value = serde_json::to_value(stored).map_err(|e| e.to_string())?;
    let object = patch.as_object().ok_or("偏好必须为对象")?;
    for (key, field) in object {
        if !value.as_object().is_some_and(|v| v.contains_key(key))
            && !matches!(
                key.as_str(),
                "active_provider"
                    | "active_model"
                    | "subagent_provider"
                    | "subagent_model"
                    | "subagent_thinking_level"
            )
        {
            return Err(format!("未知偏好：{key}"));
        }
        value[key] = field.clone();
    }
    let p: husk_host::prefs::DefaultPreferences =
        serde_json::from_value(value).map_err(|e| e.to_string())?;
    if !matches!(
        p.permission_mode.as_str(),
        "default" | "acceptEdits" | "auto" | "dontAsk" | "bypassPermissions"
    ) || !matches!(p.agent_mode.as_str(), "build" | "plan" | "goal" | "office")
        || !matches!(
            p.default_workspace_mode.as_str(),
            "last" | "build" | "office"
        )
        || !(0.5..=0.95).contains(&p.compact_at)
    {
        return Err("无效的默认偏好".into());
    }
    husk_host::prefs::save_default_preferences(&p).map_err(|e| e.to_string())?;
    manager.set_default_prefs(
        object
            .get("permission_mode")
            .and_then(Value::as_str)
            .map(str::to_owned),
        object
            .get("thinking_level")
            .map(|v| v.as_str().unwrap_or("").to_owned()),
        object
            .get("agent_mode")
            .and_then(Value::as_str)
            .map(str::to_owned),
        object
            .get("compact_at")
            .and_then(Value::as_f64)
            .map(|f| f as f32),
        object.get("approval_timeout_secs").and_then(Value::as_u64),
        object.get("question_timeout_secs").and_then(Value::as_u64),
    );
    if object.contains_key("default_workspace_mode") {
        manager.set_default_workspace_mode(p.default_workspace_mode.clone());
    }
    if object.contains_key("blueprint_enabled") {
        manager.set_blueprint_enabled(p.blueprint_enabled);
    }
    if object.contains_key("semantic_recall") {
        manager.set_semantic_recall(p.semantic_recall);
    }
    if object.contains_key("active_provider") || object.contains_key("active_model") {
        manager.set_default_model(p.active_provider.clone(), p.active_model.clone());
    }
    if object.contains_key("subagent_provider") || object.contains_key("subagent_model") {
        manager.set_subagent_model(p.subagent_pair());
    }
    if object.contains_key("subagent_thinking_level") {
        manager.set_subagent_thinking_level(p.subagent_thinking_level.clone());
    }
    let mut limits = husk_host::sandbox_prefs::current();
    limits.enabled = p.sandbox_enabled;
    limits.network = p
        .sandbox_network
        .as_deref()
        .and_then(husk_host::sandbox_prefs::network_from_label);
    limits.max_memory_mb = p.sandbox_max_memory_mb;
    limits.max_processes = p.sandbox_max_processes;
    limits.allowed_domains = p.sandbox_allowed_domains.unwrap_or_default();
    limits.denied_domains = p.sandbox_denied_domains.unwrap_or_default();
    husk_host::sandbox_prefs::set(limits);
    Ok(())
}

pub(super) fn execute(
    command: SettingsCommand,
    manager: &mut SessionManager,
    paths: &agent_paths::AppPaths,
) -> Result<(), String> {
    match command {
        SettingsCommand::Terminal(patch) => {
            crate::features::terminal::TerminalPreferences::save_patch(&paths.config_dir, patch)?;
        }
        SettingsCommand::ForkSession(key) => {
            let exists = manager
                .projects_snapshot()
                .build()
                .iter()
                .any(|p| p.root == key.root && p.sessions.iter().any(|s| s.id == key.id));
            if !exists {
                return Err("会话不存在".into());
            }
            if manager.workspace_root.to_string_lossy() != key.root || !manager.workspace_active {
                manager
                    .switch_workspace(PathBuf::from(&key.root))
                    .map_err(|e| e.to_string())?;
            }
            manager.fork_session(key.id).ok_or("无法复制会话历史")?;
        }
        SettingsCommand::SetSessionArchived { key, archived } => {
            if !manager
                .archive_session(Some(&key.root), key.id, archived)
                .map_err(|e| e.to_string())?
            {
                return Err("会话不存在".into());
            }
        }
        SettingsCommand::ExportSession { key, path } => {
            let (_, markdown) = manager
                .export_session_markdown(Some(&key.root), key.id)
                .ok_or("会话历史不可用")?;
            if path.as_os_str().is_empty() {
                return Err("请选择导出路径".into());
            }
            std::fs::write(path, markdown).map_err(|e| e.to_string())?;
        }
        SettingsCommand::RemoveWorkspace(root) => {
            if !manager.remove_workspace(&root) {
                return Err("无法移除此工作区".into());
            }
        }
        SettingsCommand::Refresh => {}
        SettingsCommand::Preferences(patch) => patch_preferences(patch, manager)?,
        SettingsCommand::Appearance(patch) => {
            let mut value = serde_json::to_value(husk_host::prefs::load_appearance_settings())
                .map_err(|e| e.to_string())?;
            for (key, field) in patch.as_object().ok_or("外观必须为对象")? {
                value[key] = field.clone();
            }
            let appearance: husk_host::prefs::AppearanceSettings =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            if !matches!(appearance.theme_mode.as_str(), "system" | "light" | "dark")
                || !matches!(appearance.currency.as_str(), "usd" | "cny")
            {
                return Err("无效的外观选项".into());
            }
            husk_host::prefs::save_appearance_settings(&appearance).map_err(|e| e.to_string())?;
        }
        SettingsCommand::SaveProvider {
            original,
            id,
            mut value,
        } => edit_config(manager, paths, |config| {
            safe_name(&id)?;
            let providers = config["providers"]
                .as_object_mut()
                .ok_or("提供商配置无效")?;
            if original.as_deref() != Some(&id) && providers.contains_key(&id) {
                return Err("提供商 ID 已存在".into());
            }
            if let Some(original) = &original {
                let stored = providers.get(original).ok_or("提供商已被删除，请刷新")?;
                restore_secrets(&mut value, stored);
                value["models"] = stored["models"].clone();
                if text(&value, "api_key").is_empty() {
                    value["api_key"] = stored["api_key"].clone();
                }
            }
            providers.insert(id.clone(), value);
            if let Some(original) = original.filter(|old| old != &id) {
                providers.remove(&original);
                if config["active_provider"] == original {
                    config["active_provider"] = json!(id);
                }
                if let Some(chain) = config["fallback_chain"].as_array_mut() {
                    for provider in chain {
                        if provider.as_str() == Some(&original) {
                            *provider = json!(id);
                        }
                    }
                }
            }
            Ok(())
        })?,
        SettingsCommand::DeleteProvider(id) => edit_config(manager, paths, |config| {
            if config["providers"]
                .as_object_mut()
                .ok_or("提供商配置无效")?
                .remove(&id)
                .is_none()
            {
                return Err("提供商不存在".into());
            }
            if config["active_provider"] == id {
                config["active_provider"] = Value::Null;
                config["active_model"] = Value::Null;
            }
            if let Some(chain) = config["fallback_chain"].as_array_mut() {
                chain.retain(|v| v.as_str() != Some(&id));
            }
            Ok(())
        })?,
        SettingsCommand::SaveModel {
            provider,
            original,
            value,
        } => edit_config(manager, paths, |config| {
            let id = text(&value, "id");
            if id.trim().is_empty() {
                return Err("模型 ID 不能为空".into());
            }
            let models = config["providers"][&provider]["models"]
                .as_array_mut()
                .ok_or("提供商不存在")?;
            let model_id = |v: &Value| v.as_str().unwrap_or_else(|| text(v, "id")).to_owned();
            if models
                .iter()
                .any(|m| model_id(m) == id && original.as_deref() != Some(id))
            {
                return Err("模型 ID 已存在".into());
            }
            if let Some(old) = original.as_deref() {
                let index = models
                    .iter()
                    .position(|m| model_id(m) == old)
                    .ok_or("模型已被删除，请刷新")?;
                models[index] = value.clone();
            } else {
                models.push(value.clone());
            }
            if config["active_provider"] == provider
                && original.as_deref() == config["active_model"].as_str()
            {
                config["active_model"] = json!(id);
            }
            Ok(())
        })?,
        SettingsCommand::DeleteModel { provider, id } => edit_config(manager, paths, |config| {
            let models = config["providers"][&provider]["models"]
                .as_array_mut()
                .ok_or("提供商不存在")?;
            let old = models.len();
            models.retain(|m| m.as_str().unwrap_or_else(|| text(m, "id")) != id);
            if old == models.len() {
                return Err("模型不存在".into());
            }
            if config["active_provider"] == provider && config["active_model"] == id {
                config["active_model"] = Value::Null;
            }
            Ok(())
        })?,
        SettingsCommand::Instructions { scope, content } => {
            manager.set_instructions(&scope, &content)?
        }
        SettingsCommand::SaveRule(v) => {
            let globs = v["globs"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            husk_host::rules::save_rule(
                &manager.workspace_root,
                paths,
                text(&v, "name"),
                v["global"].as_bool().unwrap_or(false),
                text(&v, "description"),
                &globs,
                v["always"].as_bool().unwrap_or(true),
                text(&v, "body"),
            )
            .map_err(|e| e.to_string())?;
        }
        SettingsCommand::DeleteRule { name, global } => {
            safe_name(&name)?;
            std::fs::remove_file(husk_host::rules::rule_file(
                &manager.workspace_root,
                paths,
                &name,
                global,
            ))
            .map_err(|e| e.to_string())?;
        }
        SettingsCommand::SaveSubagent(v) => {
            safe_name(text(&v, "name"))?;
            husk_host::subagents::save_manifest(
                &manager.workspace_root,
                paths,
                text(&v, "name"),
                v["global"].as_bool().unwrap_or(false),
                text(&v, "description"),
                v["model"].as_str(),
                v["thinking"].as_str(),
                v["readonly"].as_bool(),
                text(&v, "prompt"),
            )
            .map_err(|e| e.to_string())?;
        }
        SettingsCommand::DeleteSubagent { name, global } => {
            safe_name(&name)?;
            let dir = if global {
                husk_host::subagents::global_agent_dir(paths)
            } else {
                husk_host::subagents::workspace_agent_dir(&manager.workspace_root, paths)
            };
            std::fs::remove_file(dir.join(format!("{name}.md"))).map_err(|e| e.to_string())?;
        }
        SettingsCommand::SaveSkill {
            name,
            global,
            content,
        } => {
            safe_name(&name)?;
            let dir = skill_dir(manager, global)?.join(&name);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let content = if content.trim_start().starts_with("---") {
                content
            } else {
                format!("---\nname: {name}\ndescription: {name}\n---\n\n{content}\n")
            };
            std::fs::write(dir.join("SKILL.md"), content).map_err(|e| e.to_string())?;
        }
        SettingsCommand::DeleteSkill { name, global } => {
            safe_name(&name)?;
            std::fs::remove_file(skill_dir(manager, global)?.join(name).join("SKILL.md"))
                .map_err(|e| e.to_string())?;
        }
        SettingsCommand::SaveMcp { id, mut value } => {
            safe_name(&id)?;
            // MCP discovery loads PluginManifest, not a naked transport entry.
            if value.get("entry").is_none() {
                let entry = value;
                value = json!({"id": id, "name": id, "kind": "mcp", "entry": entry});
                if value["entry"].get("url").is_none() {
                    value["permissions"] = json!({});
                }
            }
            value["id"] = json!(id);
            value["kind"] = json!("mcp");
            let dir = paths.config_dir.join("mcp").join(&id);
            let file = dir.join("manifest.json");
            if file.exists() {
                let stored: Value = serde_json::from_str(
                    &std::fs::read_to_string(&file).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                restore_secrets(&mut value, &stored);
            }
            let entry = &value["entry"];
            if let Some(url) = entry["url"].as_str() {
                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    return Err("MCP URL 必须为 http(s) 地址".into());
                }
            } else if text(entry, "command").trim().is_empty() {
                return Err("MCP command 不能为空".into());
            }
            if let Some(args) = entry.get("args")
                && !args
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string))
            {
                return Err("MCP args 必须为字符串数组".into());
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(
                &file,
                serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        SettingsCommand::DeleteMcp(id) => {
            safe_name(&id)?;
            std::fs::remove_file(paths.config_dir.join("mcp").join(id).join("manifest.json"))
                .map_err(|e| e.to_string())?;
        }
        SettingsCommand::MemorySet {
            scope: which,
            key,
            value,
        } => {
            if key.trim().is_empty() {
                return Err("记忆名称不能为空".into());
            }
            memory(manager)?.set_persona_scoped(scope(&which)?, &key, &value)?;
        }
        SettingsCommand::MemoryForget { scope: which, key } => {
            memory(manager)?.remove_persona_scoped(scope(&which)?, &key)?;
        }
        SettingsCommand::MemoryRemoveFact(id) => {
            memory(manager)?.remove_fact(id)?;
        }
        SettingsCommand::MemoryClear(table) => {
            if !matches!(table.as_str(), "all" | "facts" | "episodes" | "persona") {
                return Err("无效的记忆表".into());
            }
            memory(manager)?.clear(&table)?;
        }
        SettingsCommand::SemanticRetry => {
            agent_context::memory::semantic::retry();
        }
        SettingsCommand::SemanticCancel => {
            agent_context::memory::semantic::cancel();
        }
        SettingsCommand::SemanticRemove => {
            agent_context::memory::semantic::remove_model();
        }
        SettingsCommand::FallbackChain(chain) => edit_config(manager, paths, |config| {
            let providers = config["providers"].as_object().ok_or("提供商配置无效")?;
            let mut seen = std::collections::HashSet::new();
            if chain
                .iter()
                .any(|id| !providers.contains_key(id) || !seen.insert(id))
            {
                return Err("回退链必须由已配置且不重复的提供商 ID 组成".into());
            }
            config["fallback_chain"] = json!(chain);
            Ok(())
        })?,
        SettingsCommand::PluginEnabled { id, enabled } => {
            manager.set_plugin_enabled(&id, enabled)?;
        }
        SettingsCommand::PluginTrust(id) => {
            manager.trust_plugin(&id)?;
        }
        SettingsCommand::PluginRemove(id) => {
            SessionManager::remove_plugin(&manager.workspace_root, &id, paths)?;
            manager.reload_plugins();
        }
        SettingsCommand::PluginReload => {
            manager.reload_plugins();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_cannot_escape_owned_settings_directories() {
        for name in ["", "../x", "a/b", "a\\b", "..", "."] {
            assert!(safe_name(name).is_err(), "{name}");
        }
        assert!(safe_name("my-agent_2").is_ok());
    }
    #[test]
    fn secrets_never_enter_snapshots_and_masks_roundtrip() {
        let original = json!({"api_key":"not-a-real-key","headers":{"Authorization":"not-a-real-token"},"models":[{"id":"demo"}]});
        let mut masked = original.clone();
        mask_secrets(&mut masked);
        let rendered = masked.to_string();
        assert!(!rendered.contains("not-a-real"));
        restore_secrets(&mut masked, &original);
        assert_eq!(masked, original);
    }
    #[test]
    fn masking_preserves_absent_credentials() {
        let mut value = json!({"api_key":"","headers":{}});
        mask_secrets(&mut value);
        assert_eq!(value, json!({"api_key":"","headers":{}}));
    }
}
