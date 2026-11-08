use super::*;
const APP_ID: i32 = 0x4146494f;
const MIGRATIONS: &[&str] = &[include_str!("schema.sql")];

fn schema(e: impl std::fmt::Display) -> MetadataError {
    error(MetadataErrorCode::Schema, e.to_string())
}
fn version(conn: &Connection) -> Result<usize> {
    let app: i32 = conn
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(schema)?;
    let raw: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(schema)?;
    let version = usize::try_from(raw).map_err(schema)?;
    if ((version > 0 || app != 0) && app != APP_ID) || version > MIGRATIONS.len() {
        return Err(schema(
            "Foreign registry or unsupported future schema; refusing access",
        ));
    }
    if version == 0 {
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .map_err(schema)?;
        if tables != 0 {
            return Err(schema("Unrecognized nonempty database"));
        }
    } else {
        for (index, sql) in MIGRATIONS.iter().take(version).enumerate() {
            let recorded: String = conn
                .query_row(
                    "SELECT sql FROM schema_migrations WHERE version=?1",
                    [(index + 1) as i64],
                    |r| r.get(0),
                )
                .map_err(schema)?;
            if canonical_sql(&recorded) != canonical_sql(sql) {
                return Err(schema("Migration history differs from this application"));
            }
        }
    }
    Ok(version)
}

pub(super) fn validate(conn: &Connection) -> Result<()> {
    if version(conn)? != MIGRATIONS.len() {
        return Err(schema("Registry needs migration by an explicit save"));
    }
    Ok(())
}

pub(super) fn migrate(conn: &mut Connection, path: &Path) -> Result<()> {
    let prior = version(conn)?;
    if prior == MIGRATIONS.len() {
        return Ok(());
    }
    // No backup is needed for an empty registry. When future migrations are
    // added, preserve the existing live database with SQLite's backup API.
    if prior > 0 {
        let backup = path.with_extension(format!(
            "before-v{}-{}.sqlite3",
            prior + 1,
            uuid::Uuid::new_v4()
        ));
        conn.backup("main", backup, rusqlite::backup::NO_PROGRESS)
            .map_err(storage)?;
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    // Recheck under the writer lock; another process may have migrated first.
    let current = version(&tx)?;
    if current == 0 {
        tx.execute_batch(
            "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, sql TEXT NOT NULL)",
        )
        .map_err(schema)?;
    }
    apply(&tx, current, MIGRATIONS)?;
    tx.pragma_update(None, "application_id", APP_ID)
        .map_err(schema)?;
    tx.commit().map_err(storage)
}

fn apply(tx: &rusqlite::Transaction<'_>, current: usize, migrations: &[&str]) -> Result<()> {
    for (index, sql) in migrations.iter().enumerate().skip(current) {
        tx.execute_batch(sql).map_err(schema)?;
        tx.execute(
            "INSERT INTO schema_migrations(version,sql) VALUES(?1,?2)",
            params![(index + 1) as i64, canonical_sql(sql)],
        )
        .map_err(schema)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)
            .map_err(schema)?;
    }
    Ok(())
}

fn canonical_sql(sql: &str) -> String {
    sql.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_migration_rolls_back_ddl_and_version() {
        let mut c = Connection::open_in_memory().unwrap();
        {
            let tx = c.transaction().unwrap();
            tx.execute_batch(
                "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, sql TEXT NOT NULL)",
            )
            .unwrap();
            assert!(
                apply(
                    &tx,
                    0,
                    &["CREATE TABLE partial(id INTEGER);", "INVALID SQL"]
                )
                .is_err()
            );
        }
        let count: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(version(&c).unwrap(), 0);
    }
}
