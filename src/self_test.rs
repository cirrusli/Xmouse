use anyhow::{Result, ensure};
use rusqlite::{Connection, params};

/// Packaging probe: runs before COM, singleton, user data, UI or input hooks.
pub fn run() -> Result<()> {
    let mut connection = Connection::open_in_memory()?;
    connection.execute_batch("CREATE TABLE probe (text TEXT NOT NULL, image BLOB NOT NULL);")?;
    let transaction = connection.transaction()?;
    let text = "Xmouse 独立运行 · 中文与 spaces";
    let bytes = [0u8, 1, 127, 255];
    transaction.execute(
        "INSERT INTO probe VALUES (?1, ?2)",
        params![text, &bytes[..]],
    )?;
    transaction.commit()?;
    let (restored_text, restored_bytes): (String, Vec<u8>) =
        connection.query_row("SELECT text, image FROM probe", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    ensure!(
        restored_text == text && restored_bytes == bytes,
        "SQLite round trip failed"
    );
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(
        integrity == "ok",
        "SQLite integrity check failed: {integrity}"
    );
    println!(
        "Xmouse {} self-test OK; SQLite {}",
        env!("CARGO_PKG_VERSION"),
        rusqlite::version()
    );
    Ok(())
}
