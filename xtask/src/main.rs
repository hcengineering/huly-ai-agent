use std::env;

use clap::Parser;

type DynError = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    if let Err(e) = try_main().await {
        eprintln!("{e}");
        std::process::exit(-1);
    }
}

async fn try_main() -> Result<(), DynError> {
    let task = env::args().nth(1);
    match task.as_deref() {
        Some("sqlx") => sqlx().await?,
        _ => print_help(),
    }
    Ok(())
}

fn print_help() {
    eprintln!(
        "Tasks:

sqlx <args>  run sqlx command with correct context
"
    )
}

async fn sqlx() -> Result<(), DynError> {
    std::fs::create_dir_all("data").ok();
    unsafe {
        libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut libsqlite3_sys::sqlite3,
                *mut *mut i8,
                *const libsqlite3_sys::sqlite3_api_routines,
            ) -> i32,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    }

    let opt = sqlx_cli::Opt::parse_from(env::args().skip(1));
    sqlx_cli::run(opt).await?;
    Ok(())
}
