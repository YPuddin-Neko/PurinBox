//! Python 子进程共享基础设施
//!
//! 五个 AI 功能（打标 / 超分 / 人物裁切 / 美学评分 / 聚类）都以子进程方式调用
//! 打包在 scripts/ 下的 Python 脚本，此模块收敛两块原先各自复制的逻辑：
//! 1. 推理脚本路径解析（开发 / exe 同级 scripts / NSIS exe 同级 / macOS Resources）
//! 2. Windows 下 CUDA/cuDNN DLL 的 PATH 注入（进程环境 + 注册表回退 + cuDNN 9.x 子目录）

use std::path::PathBuf;
use std::process::Command;

/// 解析打包的 Python 脚本路径。
///
/// 按顺序尝试四个候选位置，全部未命中时在错误信息中列出搜索路径。
pub fn find_script(script_name: &str) -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let candidates = vec![
        // 开发模式: CARGO_MANIFEST_DIR/scripts/
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("scripts/{}", script_name)),
        // 生产模式 Windows/Linux: exe 同级 scripts/
        exe_dir.join(format!("scripts/{}", script_name)),
        // 生产模式 Windows NSIS: exe 同级
        exe_dir.join(script_name),
        // macOS .app bundle: Resources/scripts/
        exe_dir.join(format!("../Resources/scripts/{}", script_name)),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.canonicalize().unwrap_or_else(|_| path.clone()));
        }
    }

    let paths_str = candidates
        .iter()
        .enumerate()
        .map(|(i, p)| format!("  {}. {}", i + 1, p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "推理脚本 {} 未找到。\n搜索路径:\n{}",
        script_name, paths_str
    ))
}

/// 为 Python 子进程应用平台配置：
/// - Windows: CREATE_NO_WINDOW；use_gpu 时把 CUDA/cuDNN DLL 目录注入 PATH
/// - 其他平台: 无操作
#[cfg(target_os = "windows")]
pub fn configure_python_command(cmd: &mut Command, use_gpu: bool) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    if use_gpu {
        cmd.env("PATH", build_cuda_enhanced_path());
    }
}

#[cfg(not(target_os = "windows"))]
pub fn configure_python_command(cmd: &mut Command, _use_gpu: bool) {
    // 让子进程自成进程组：kill_process_tree 的 `kill -9 -PID` 需要 PGID==PID 才能
    // 连同 torch/onnxruntime 派生的 worker 一起杀掉，否则永远走单杀回退留下孤儿
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

/// 从 Windows 注册表读取系统环境变量（GUI 进程可能没有最新环境变量）
#[cfg(target_os = "windows")]
fn read_env_from_registry(name: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    // 使用 reg query 读取系统环境变量
    let output = Command::new("reg")
        .args([
            "query",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            "/v",
            name,
        ])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // 格式: "    CUDA_PATH    REG_SZ    J:\NVIDIA\CUDA"
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with(name) {
            // 按空白分割，取最后一个值
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                return Some(parts[2..].join(" "));
            }
        }
    }
    // 用户环境变量
    let output = Command::new("reg")
        .args(["query", r"HKCU\Environment", "/v", name])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with(name) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                return Some(parts[2..].join(" "));
            }
        }
    }
    None
}

/// 获取 CUDA 相关环境变量（进程环境 + 注册表补充）
#[cfg(target_os = "windows")]
pub fn get_cuda_env_vars() -> Vec<(String, String)> {
    let mut result: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    // 1. 从进程环境变量获取
    for (key, val) in std::env::vars() {
        if key == "CUDA_PATH" || key.starts_with("CUDA_PATH_V") || key == "CUDA_HOME" {
            result.insert(key, val);
        }
    }

    // 2. 如果进程中没有 CUDA_PATH，尝试从注册表读取
    if !result.contains_key("CUDA_PATH") {
        if let Some(val) = read_env_from_registry("CUDA_PATH") {
            result.insert("CUDA_PATH".to_string(), val);
        }
    }
    // 注册表中的 CUDA_PATH_V* 变量
    for suffix in &[
        "V12_9", "V12_8", "V12_6", "V12_4", "V12_2", "V12_1", "V12_0",
    ] {
        let key = format!("CUDA_PATH_{}", suffix);
        if !result.contains_key(&key) {
            if let Some(val) = read_env_from_registry(&key) {
                result.insert(key, val);
            }
        }
    }

    result.into_iter().collect()
}

/// 将目录下的所有子目录添加到 PATH 字符串中（用于 cuDNN 9.x 的 bin/12.x 结构）
#[cfg(target_os = "windows")]
fn add_subdirs_to_path(dir: &str, path: &mut String) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let sub = entry.path();
                let sub_str = sub.to_string_lossy().to_string();
                if !path.contains(&sub_str) {
                    *path = format!("{};{}", sub_str, path);
                }
            }
        }
    }
}

/// 构建注入了 CUDA/cuDNN DLL 目录的 PATH 值
#[cfg(target_os = "windows")]
fn build_cuda_enhanced_path() -> String {
    let mut path = std::env::var("PATH").unwrap_or_default();

    // 辅助闭包：目录存在且未加入时前置到 PATH
    let mut add_dir = |dir: &str| {
        if std::path::Path::new(dir).exists() && !path.contains(dir) {
            path = format!("{};{}", dir, path);
        }
    };

    // 1. CUDA 路径：从环境变量读取（含注册表回退）
    for (_key, val) in get_cuda_env_vars() {
        let bin = format!(r"{}\bin", val);
        let bin_x64 = format!(r"{}\bin\x64", val); // cuDNN 9.x
        let lib = format!(r"{}\lib\x64", val);
        add_dir(&bin);
        add_dir(&bin_x64);
        add_dir(&lib);
    }

    // 2. cuDNN 路径：CUDNN_PATH 可能指向独立安装目录
    if let Ok(cudnn_path) = std::env::var("CUDNN_PATH") {
        let bin = format!(r"{}\bin", cudnn_path);
        add_dir(&bin);
        // cuDNN 9.x 在 bin/lib 下有 12.x 子目录
        add_subdirs_to_path(&bin, &mut path);
        let lib = format!(r"{}\lib", cudnn_path);
        add_subdirs_to_path(&lib, &mut path);
    }

    // 3. 扫描 PATH 中已含 cuDNN DLL 的目录，自动加其子目录（cuDNN 9.x 结构）
    let current_path = path.clone();
    for dir in current_path.split(';') {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let has_cudnn = entries.into_iter().flatten().any(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name.contains("cudnn") && name.ends_with(".dll")
            });
            if has_cudnn {
                add_subdirs_to_path(dir, &mut path);
            }
        }
    }

    path
}
