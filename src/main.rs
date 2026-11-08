use std::process::ExitCode;

use agent_store::cli::{self, DATABASE_ENV, Environment};

fn main() -> ExitCode {
    let environment = Environment {
        database: std::env::var_os(DATABASE_ENV).map(Into::into),
        data_dir: dirs::data_local_dir(),
        cwd: std::env::current_dir().ok(),
        now: chrono::Utc::now(),
    };
    cli::run(
        std::env::args_os(),
        &environment,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
    )
}
