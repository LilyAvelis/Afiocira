//! Download only flavor data from a pinned Git object; never check out or run repo code.
use super::*;
use std::fs;
use std::os::windows::process::CommandExt;
use std::process::{Command as Process, Stdio};

fn git(args: &[String], timeout: Duration) -> Result<Vec<u8>, String> {
    let mut child = Process::new("git")
        .creation_flags(0x0800_0000)
        .args([
            "-c",
            "core.hooksPath=NUL",
            "-c",
            "protocol.file.allow=never",
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let mut stdout = child.stdout.take().ok_or("Missing Git output")?;
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut bytes = Vec::new();
        stdout
            .by_ref()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let output = reader
                .join()
                .map_err(|_| "Git output worker failed")?
                .map_err(|e| e.to_string())?;
            return if status.success() {
                Ok(output)
            } else {
                Err("GitHub flavor download failed".into())
            };
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Flavor download timed out".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
fn text(args: &[String]) -> Result<String, String> {
    String::from_utf8(git(args, Duration::from_secs(30))?).map_err(|e| e.to_string())
}
fn source_parts(source: &str) -> Result<(String, String, String), String> {
    let source = source
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let (repository, subdir) = source.split_once(':').unwrap_or((source, ""));
    let parts: Vec<_> = repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        })
        || subdir.split('/').any(|p| {
            p == ".."
                || (!p.is_empty()
                    && !p
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
        })
    {
        return Err("Use owner/repository or owner/repository:flavor-subdirectory".into());
    }
    let name = if subdir.is_empty() {
        parts[1]
    } else {
        subdir.rsplit('/').next().unwrap()
    }
    .trim_end_matches(".yazi");
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Flavor name must use letters, numbers or hyphens".into());
    }
    Ok((repository.into(), subdir.into(), name.into()))
}
pub fn install(source: &str) -> Result<String, String> {
    let (repository, subdir, name) = source_parts(source)?;
    let root = yazi_profile::root()?;
    let destination = root.join("flavors").join(format!("{name}.yazi"));
    if destination.exists() {
        return Err("A flavor with this name is already installed".into());
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let staging = root.join(format!(
        "download-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let repo = staging.join("repo");
        let dir = staging.join(format!("{name}.yazi"));
        fs::create_dir(&dir).map_err(|e| e.to_string())?;
        git(
            &[
                "clone".into(),
                "--quiet".into(),
                "--depth=1".into(),
                "--filter=blob:none".into(),
                "--no-checkout".into(),
                format!("https://github.com/{repository}.git"),
                repo.to_string_lossy().into(),
            ],
            Duration::from_secs(60),
        )?;
        let prefix = vec!["-C".to_owned(), repo.to_string_lossy().into_owned()];
        let revision = text(&[prefix.clone(), vec!["rev-parse".into(), "HEAD".into()]].concat())?
            .trim()
            .to_owned();
        for file in [
            "flavor.toml",
            "tmtheme.xml",
            "README.md",
            "LICENSE",
            "LICENSE-tmtheme",
            "preview.png",
        ] {
            let object = format!(
                "HEAD:{}{file}",
                if subdir.is_empty() {
                    String::new()
                } else {
                    format!("{subdir}/")
                }
            );
            let size = text(
                &[
                    prefix.clone(),
                    vec!["cat-file".into(), "-s".into(), object.clone()],
                ]
                .concat(),
            );
            if size.is_err() && matches!(file, "README.md" | "preview.png") {
                continue;
            }
            let size = size?.trim().parse::<usize>().map_err(|e| e.to_string())?;
            if size > 4 * 1024 * 1024 {
                return Err("Flavor file exceeds 4 MiB".into());
            }
            let bytes = git(
                &[prefix.clone(), vec!["show".into(), object]].concat(),
                Duration::from_secs(30),
            )?;
            if bytes.len() != size {
                return Err("Incomplete flavor download".into());
            }
            fs::write(dir.join(file), bytes).map_err(|e| e.to_string())?;
        }
        let license = fs::read_to_string(dir.join("LICENSE")).map_err(|e| e.to_string())?;
        if ![
            "MIT License",
            "Apache License",
            "BSD",
            "ISC License",
            "GNU GENERAL PUBLIC LICENSE",
        ]
        .iter()
        .any(|known| license.contains(known))
        {
            return Err("License is not recognized as a supported open-source license".into());
        }
        fs::write(dir.join("afio-source.json"), serde_json::to_vec_pretty(&serde_json::json!({"source":source,"revision":revision,"yazi":yazi_profile::VERSION})).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        yazi_profile::validate_flavor(&dir)?;
        fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::rename(dir, &destination).map_err(|e| e.to_string())?;
        Ok(name)
    })();
    // The staging path is created directly under our own settings directory.
    if let (Ok(parent), Ok(target)) = (root.canonicalize(), staging.canonicalize())
        && target.parent() == Some(parent.as_path())
        && target != parent
    {
        let _ = fs::remove_dir_all(target);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_is_a_github_data_repository_not_an_arbitrary_command_or_path() {
        assert!(source_parts("yazi-rs/flavors:catppuccin-mocha.yazi").is_ok());
        for source in [
            "../secret",
            "file:///private/repo",
            "owner/repo:../../private",
            "owner/repo;evil",
        ] {
            assert!(source_parts(source).is_err());
        }
    }
}
