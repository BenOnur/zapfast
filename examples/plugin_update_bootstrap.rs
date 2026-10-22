//! Verify a plugin-channel package, or bootstrap it through the real updater.
//! Run a copy outside Cargo's target directory, beside zapfast-portable.txt.
use std::io::Write;

const CONFIG: fastframe_update::UpdateConfig = fastframe_update::UpdateConfig {
    current_version: "0.19.0",
    ..zapfast::updates::CONFIG
};

fn main() -> anyhow::Result<()> {
    let launch = fastframe_update::intercept(&CONFIG);
    let arguments: Vec<_> = launch
        .arguments
        .iter()
        .map(|a| a.to_string_lossy())
        .collect();
    if arguments.iter().any(|a| a == "--version") {
        writeln!(std::io::stdout(), "zapfast {}", CONFIG.current_version)?;
        return Ok(());
    }
    if launch.error.is_some() {
        anyhow::bail!("The update failed to start; the bootstrap was restored");
    }
    let client = reqwest::blocking::Client::builder();
    let updater =
        fastframe_update::Updater::new(CONFIG, fastframe_update::ReqwestTransport::new(client)?);
    let release = updater
        .check()?
        .ok_or_else(|| anyhow::anyhow!("No newer plugin package"))?;
    let prepared = updater.download(&release, |_, _| {})?;
    writeln!(
        std::io::stdout(),
        "Verified signed plugin package {}",
        prepared.version()
    )?;
    if arguments.iter().any(|a| a == "--install") {
        updater.handoff(prepared, Vec::new())?;
    } else {
        prepared.discard();
    }
    Ok(())
}
