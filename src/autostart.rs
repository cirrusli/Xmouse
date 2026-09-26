use anyhow::{Context, Result, bail};
use std::{
    env,
    ffi::OsStr,
    fs, mem,
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{self, Command},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND, GetLastError, WAIT_OBJECT_0},
    System::{
        Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
        },
        Threading::{CREATE_NO_WINDOW, GetExitCodeProcess, INFINITE, WaitForSingleObject},
    },
    UI::{
        Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};

const TASK_NAME: &str = "Xmouse Elevated Autostart";
const INSTALL_HELPER_ARGUMENT: &str = "--install-elevated-autostart";
const REMOVE_HELPER_ARGUMENT: &str = "--remove-elevated-autostart";
const LEGACY_RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const LEGACY_VALUE_NAME: &str = "Xmouse";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HelperCommand {
    Install,
    Remove,
}

impl HelperCommand {
    fn from_argument(argument: &OsStr) -> Option<Self> {
        if argument == INSTALL_HELPER_ARGUMENT {
            Some(Self::Install)
        } else if argument == REMOVE_HELPER_ARGUMENT {
            Some(Self::Remove)
        } else {
            None
        }
    }

    fn argument(self) -> &'static str {
        match self {
            Self::Install => INSTALL_HELPER_ARGUMENT,
            Self::Remove => REMOVE_HELPER_ARGUMENT,
        }
    }
}

/// Runs a privileged one-shot helper before the normal single-instance guard.
/// `None` means this is a normal Xmouse launch.
pub fn handle_helper_command() -> Option<Result<()>> {
    let command = env::args_os()
        .skip(1)
        .find_map(|argument| HelperCommand::from_argument(&argument));
    command.map(|command| match command {
        HelperCommand::Install => install_task(),
        HelperCommand::Remove => remove_task(),
    })
}

/// Synchronizes the external startup mechanism when settings are saved.
///
/// Existing elevated tasks are left alone on ordinary saves so UAC is not
/// requested repeatedly. A missing task is repaired the next time settings are
/// saved while the switch remains enabled.
pub fn apply_setting(enabled: bool, was_enabled: bool) -> Result<()> {
    let task_exists = is_task_registered();
    if enabled {
        if !was_enabled || !task_exists {
            run_elevated_helper(HelperCommand::Install)?;
        }
    } else if task_exists {
        run_elevated_helper(HelperCommand::Remove)?;
    }

    remove_legacy_run_value().context("清理旧版开机启动项失败")?;
    Ok(())
}

pub fn is_task_registered() -> bool {
    let mut command = schtasks_command();
    command.args(["/Query", "/TN", TASK_NAME]);
    command.status().is_ok_and(|status| status.success())
}

pub fn record_helper_error(message: &str) {
    let path = helper_error_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, message);
}

fn install_task() -> Result<()> {
    let executable = env::current_exe().context("无法定位 Xmouse 可执行文件")?;
    let user_sid = current_user_sid()?;
    let task_xml = task_xml(&executable, &user_sid);
    let xml_path = env::temp_dir().join(format!("xmouse-elevated-autostart-{}.xml", process::id()));
    write_utf16_file(&xml_path, &task_xml).context("无法写入临时计划任务定义")?;
    let mut command = schtasks_command();
    let output_result = command
        .args(["/Create", "/TN", TASK_NAME, "/XML"])
        .arg(&xml_path)
        .arg("/F")
        .output();
    let _ = fs::remove_file(&xml_path);
    let output = output_result.context("无法启动 Windows 计划任务工具")?;
    if !output.status.success() {
        bail!(
            "创建管理员开机启动任务失败（退出码 {:?}）：{}",
            output.status.code(),
            command_error_text(&output.stdout, &output.stderr)
        );
    }
    Ok(())
}

fn remove_task() -> Result<()> {
    if !is_task_registered() {
        return Ok(());
    }
    let mut command = schtasks_command();
    let output = command
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output()
        .context("无法启动 Windows 计划任务工具")?;
    if !output.status.success() {
        bail!(
            "删除管理员开机启动任务失败（退出码 {:?}）：{}",
            output.status.code(),
            command_error_text(&output.stdout, &output.stderr)
        );
    }
    Ok(())
}

fn run_elevated_helper(command: HelperCommand) -> Result<()> {
    let helper_error = helper_error_path();
    let _ = fs::remove_file(&helper_error);
    let executable = env::current_exe().context("无法定位 Xmouse 可执行文件")?;
    let verb = wide("runas");
    let file = wide_os(executable.as_os_str());
    let parameters = wide(command.argument());
    let directory = executable.parent().map(wide_path).unwrap_or_default();
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: ptr::null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: if directory.is_empty() {
            ptr::null()
        } else {
            directory.as_ptr()
        },
        nShow: SW_HIDE,
        ..Default::default()
    };

    let launched = unsafe { ShellExecuteExW(&mut execute) };
    if launched == 0 {
        let error = unsafe { GetLastError() };
        if error == ERROR_CANCELLED {
            bail!("已取消管理员授权，开机启动设置未更改");
        }
        bail!("无法启动管理员设置助手（错误 {error}）");
    }
    if execute.hProcess.is_null() {
        bail!("管理员设置助手未返回进程句柄");
    }

    let wait_result = unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    if wait_result != WAIT_OBJECT_0 {
        unsafe { CloseHandle(execute.hProcess) };
        bail!("等待管理员设置助手失败（结果 {wait_result}）");
    }
    let mut exit_code = 1u32;
    let read_exit_code = unsafe { GetExitCodeProcess(execute.hProcess, &mut exit_code) };
    unsafe { CloseHandle(execute.hProcess) };
    if read_exit_code == 0 {
        bail!("无法读取管理员设置助手的退出码");
    }
    if exit_code != 0 {
        let detail = fs::read_to_string(&helper_error)
            .ok()
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| format!("退出码 {exit_code}"));
        bail!("管理员设置助手执行失败：{detail}");
    }
    let _ = fs::remove_file(helper_error);
    Ok(())
}

fn current_user_sid() -> Result<String> {
    let mut command = system_command("whoami.exe");
    let output = command
        .args(["/USER", "/FO", "CSV", "/NH"])
        .output()
        .context("无法查询当前 Windows 用户 SID")?;
    if !output.status.success() {
        bail!(
            "查询当前 Windows 用户 SID 失败：{}",
            command_error_text(&output.stdout, &output.stderr)
        );
    }
    parse_user_sid(&output.stdout)
}

fn parse_user_sid(output: &[u8]) -> Result<String> {
    let marker = b"S-1-";
    let Some(start) = output
        .windows(marker.len())
        .position(|window| window == marker)
    else {
        bail!("Windows 未返回有效的当前用户 SID");
    };
    let sid = output[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit() || **byte == b'-' || **byte == b'S')
        .copied()
        .collect::<Vec<_>>();
    if sid.len() <= marker.len() {
        bail!("Windows 返回的当前用户 SID 不完整");
    }
    String::from_utf8(sid).context("当前用户 SID 编码无效")
}

fn task_xml(executable: &Path, user_sid: &str) -> String {
    let command = xml_escape(&executable.to_string_lossy());
    let working_directory = executable
        .parent()
        .map(|path| xml_escape(&path.to_string_lossy()))
        .unwrap_or_default();
    let user_sid = xml_escape(user_sid);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Xmouse elevated startup for the current interactive user.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user_sid}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user_sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>--startup</Arguments>
      <WorkingDirectory>{working_directory}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn schtasks_command() -> Command {
    system_command("schtasks.exe")
}

fn system_command(name: &str) -> Command {
    let executable = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("System32").join(name))
        .unwrap_or_else(|| PathBuf::from(name));
    let mut command = Command::new(executable);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn write_utf16_file(path: &Path, value: &str) -> Result<()> {
    let mut bytes = Vec::with_capacity(value.len() * 2 + 2);
    bytes.extend_from_slice(&[0xff, 0xfe]);
    for code_unit in value.encode_utf16() {
        bytes.extend_from_slice(&code_unit.to_le_bytes());
    }
    fs::write(path, bytes).with_context(|| format!("无法写入 {}", path.display()))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn command_error_text(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let message = format!("{} {}", stdout.trim(), stderr.trim());
    let message = message.trim();
    if message.is_empty() {
        "Windows 未返回详细错误".to_string()
    } else {
        message.to_string()
    }
}

fn helper_error_path() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("Xmouse")
        .join("autostart-helper-error.txt")
}

fn remove_legacy_run_value() -> Result<()> {
    let key_path = wide(LEGACY_RUN_KEY);
    let value_name = wide(LEGACY_VALUE_NAME);
    let mut key: HKEY = ptr::null_mut();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if opened == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if opened != 0 {
        bail!("无法打开旧版开机启动注册表项（错误 {opened}）");
    }
    let removed = unsafe { RegDeleteValueW(key, value_name.as_ptr()) };
    unsafe { RegCloseKey(key) };
    if removed != 0 && removed != ERROR_FILE_NOT_FOUND {
        bail!("无法删除旧版开机启动注册表项（错误 {removed}）");
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn wide_path(value: &Path) -> Vec<u16> {
    wide_os(value.as_os_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_argument_parser_only_accepts_internal_commands() {
        assert_eq!(
            HelperCommand::from_argument(OsStr::new(INSTALL_HELPER_ARGUMENT)),
            Some(HelperCommand::Install)
        );
        assert_eq!(
            HelperCommand::from_argument(OsStr::new(REMOVE_HELPER_ARGUMENT)),
            Some(HelperCommand::Remove)
        );
        assert_eq!(HelperCommand::from_argument(OsStr::new("--startup")), None);
    }

    #[test]
    fn task_xml_is_user_scoped_elevated_and_battery_safe() {
        let executable = Path::new(r"C:\Users\Example & User\Xmouse\Xmouse.exe");
        let xml = task_xml(executable, "S-1-5-21-123-456-789-1001");
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew"));
        assert!(xml.contains("C:\\Users\\Example &amp; User\\Xmouse\\Xmouse.exe"));
        assert_eq!(xml.matches("S-1-5-21-123-456-789-1001").count(), 2);
    }

    #[test]
    fn parses_sid_from_whoami_csv_without_decoding_account_name() {
        let output =
            b"\"CIRRUS\\Windows 11\",\"S-1-5-21-1081478708-577408454-2346103973-1001\"\r\n";
        assert_eq!(
            parse_user_sid(output).unwrap(),
            "S-1-5-21-1081478708-577408454-2346103973-1001"
        );
    }
}
