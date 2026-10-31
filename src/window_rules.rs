use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloseMethod {
    CtrlW,
    Window,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloseRule {
    pub process: String,
    pub method: CloseMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePolicy {
    CtrlW,
    Window,
    OwnerWindow,
}

impl ClosePolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::CtrlW => "Ctrl+W",
            Self::Window => "关闭当前窗口",
            Self::OwnerWindow => "关闭应用窗口",
        }
    }
}

pub fn resolve(process: &str, rules: &[CloseRule]) -> ClosePolicy {
    if let Some(rule) = rules
        .iter()
        .find(|rule| rule.process.eq_ignore_ascii_case(process))
    {
        return match rule.method {
            CloseMethod::CtrlW => ClosePolicy::CtrlW,
            CloseMethod::Window => ClosePolicy::Window,
        };
    }
    if process.eq_ignore_ascii_case("steam.exe")
        || process.eq_ignore_ascii_case("steamwebhelper.exe")
    {
        ClosePolicy::Window
    } else if process.eq_ignore_ascii_case("wallpaperui.exe") {
        ClosePolicy::OwnerWindow
    } else {
        ClosePolicy::CtrlW
    }
}

pub fn validate(rules: &[CloseRule]) -> Result<()> {
    if rules.len() > 128 {
        bail!("应用关闭规则最多 128 项");
    }
    for (index, rule) in rules.iter().enumerate() {
        let name = &rule.process;
        if name.trim() != name
            || name.len() > 260
            || name.len() <= 4
            || !name.to_ascii_lowercase().ends_with(".exe")
            || name
                .chars()
                .any(|c| "\\/:*?\"<>|".contains(c) || c.is_control())
            || rules[..index]
                .iter()
                .any(|previous| previous.process.eq_ignore_ascii_case(name))
        {
            bail!("无效或重复的应用关闭规则：{name}");
        }
    }
    Ok(())
}

pub fn set_override(
    rules: &mut Vec<CloseRule>,
    process: &str,
    method: Option<CloseMethod>,
) -> Result<()> {
    let mut next = rules.clone();
    next.retain(|rule| !rule.process.eq_ignore_ascii_case(process));
    if let Some(method) = method {
        next.push(CloseRule {
            process: process.to_ascii_lowercase(),
            method,
        });
    }
    validate(&next)?;
    next.sort_by(|left, right| left.process.cmp(&right.process));
    *rules = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_are_specific_and_an_override_can_be_reverted() {
        let mut rules = Vec::new();
        assert_eq!(resolve("STEAMWEBHELPER.EXE", &rules), ClosePolicy::Window);
        assert_eq!(resolve("wallpaperui.exe", &rules), ClosePolicy::OwnerWindow);
        for process in [
            "GTA5.exe",
            "steamservice.exe",
            "GameOverlayUI.exe",
            "msedge.exe",
        ] {
            assert_eq!(resolve(process, &rules), ClosePolicy::CtrlW);
        }
        set_override(&mut rules, "STEAMWEBHELPER.EXE", Some(CloseMethod::CtrlW)).unwrap();
        assert_eq!(resolve("steamwebhelper.exe", &rules), ClosePolicy::CtrlW);
        set_override(&mut rules, "steamwebhelper.exe", None).unwrap();
        assert_eq!(resolve("steamwebhelper.exe", &rules), ClosePolicy::Window);
    }
    #[test]
    fn invalid_and_duplicate_rules_do_not_modify_the_existing_rules() {
        let mut rules = vec![CloseRule {
            process: "editor.exe".into(),
            method: CloseMethod::Window,
        }];
        let original = rules.clone();
        assert!(set_override(&mut rules, "C:/app.exe", Some(CloseMethod::Window)).is_err());
        assert_eq!(rules, original);
        rules.push(CloseRule {
            process: "EDITOR.exe".into(),
            method: CloseMethod::CtrlW,
        });
        assert!(validate(&rules).is_err());
    }
}
