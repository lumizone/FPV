use rusqlite::{params, Connection};

use crate::error::{AppError, AppResult};

const SECONDS_PER_DAY: u64 = 86_400;
const INPUT_MICROUSD_PER_TOKEN: i64 = 3;
const OUTPUT_MICROUSD_PER_TOKEN: i64 = 15;

#[derive(Debug, Clone)]
pub struct Reservation {
    pub request_id: String,
    pub reserved_microusd: i64,
    input_microusd: i64,
}

fn day_start(now: u64) -> i64 {
    (now - now % SECONDS_PER_DAY) as i64
}

fn limit_microusd(value: &str) -> Option<i64> {
    let value = value.trim().parse::<f64>().ok()?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    // Round down the configured cap, never up, so floating-point conversion
    // cannot authorize spend above the user's requested dollar amount.
    Some((value * 1_000_000.0).floor() as i64)
}

pub fn reserve(
    conn: &mut Connection,
    request_id: &str,
    limit_usd: &str,
    prompt_bytes: usize,
    max_output_tokens: i64,
    now: u64,
) -> AppResult<Reservation> {
    let limit = limit_microusd(limit_usd)
        .filter(|limit| *limit > 0)
        .ok_or_else(|| {
            AppError::Config("cloud_daily_limit_usd must be a positive dollar amount".into())
        })?;
    let input_tokens = i64::try_from(prompt_bytes)
        .map_err(|_| AppError::Config("narration prompt is too large to price".into()))?;
    let output_tokens = max_output_tokens.max(0);
    // Byte count is an upper bound for input tokens. Reserving the requested
    // output maximum prevents concurrent requests from collectively exceeding
    // the daily cap before either provider response is known.
    let input_microusd = input_tokens
        .checked_mul(INPUT_MICROUSD_PER_TOKEN)
        .ok_or_else(|| AppError::Config("cloud narration estimate is too large".into()))?;
    let reserve = input_microusd
        .checked_add(
            output_tokens
                .checked_mul(OUTPUT_MICROUSD_PER_TOKEN)
                .ok_or_else(|| AppError::Config("cloud narration estimate is too large".into()))?,
        )
        .ok_or_else(|| AppError::Config("cloud narration estimate is too large".into()))?;
    let day = day_start(now);
    let transaction = conn.transaction()?;
    let spent: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(CASE WHEN status = 'reserved' THEN reserved_microusd ELSE charged_microusd END), 0)
         FROM cloud_daily_spend_ledger WHERE day_start = ?1",
        params![day],
        |row| row.get(0),
    )?;
    if spent
        .checked_add(reserve)
        .filter(|total| *total <= limit)
        .is_none()
    {
        return Err(AppError::Config(format!(
            "daily cloud limit reached (${:.4} reserved / ${:.2} limit)",
            spent as f64 / 1_000_000.0,
            limit as f64 / 1_000_000.0,
        )));
    }
    transaction.execute(
        "INSERT INTO cloud_daily_spend_ledger
         (request_id, day_start, reserved_microusd, charged_microusd, status, created_at)
         VALUES (?1, ?2, ?3, ?3, 'reserved', ?4)",
        params![request_id, day, reserve, now as i64],
    )?;
    transaction.commit()?;
    Ok(Reservation {
        request_id: request_id.to_owned(),
        reserved_microusd: reserve,
        input_microusd,
    })
}

pub fn complete(
    conn: &Connection,
    reservation: Reservation,
    output_tokens: usize,
    now: u64,
) -> AppResult<()> {
    let output_tokens = i64::try_from(output_tokens)
        .map_err(|_| AppError::Config("cloud response is too large to price".into()))?;
    let actual = output_tokens
        .checked_mul(OUTPUT_MICROUSD_PER_TOKEN)
        .ok_or_else(|| AppError::Config("cloud response estimate is too large".into()))?;
    // The input portion is already included in the reservation. A provider
    // response cannot justify charging more than the conservative reservation.
    let charged = reservation
        .input_microusd
        .saturating_add(actual)
        .min(reservation.reserved_microusd);
    let changed = conn.execute(
        "UPDATE cloud_daily_spend_ledger
         SET charged_microusd = ?1, status = 'completed', settled_at = ?2
         WHERE request_id = ?3 AND status = 'reserved'",
        params![charged, now as i64, reservation.request_id],
    )?;
    if changed != 1 {
        return Err(AppError::Other("cloud spend reservation was already settled or missing".into()));
    }
    Ok(())
}

/// Remove settled ledger rows older than `retention_days` so the table
/// cannot grow without bound. Called at every startup. Settled rows no
/// longer affect the daily cap (their day already passed), so pruning is
/// safe; `reserved` rows are never touched.
pub fn prune_settled(conn: &Connection, now: u64) -> AppResult<()> {
    const RETENTION_DAYS: i64 = 30;
    let cutoff = now as i64 - RETENTION_DAYS * 86_400;
    conn.execute(
        "DELETE FROM cloud_daily_spend_ledger
         WHERE status IN ('completed', 'cancelled', 'failed')
           AND settled_at IS NOT NULL
           AND settled_at < ?1",
        params![cutoff],
    )?;
    Ok(())
}

pub fn retain_reservation(
    conn: &Connection,
    reservation: Reservation,
    status: &str,
    now: u64,
) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE cloud_daily_spend_ledger
         SET status = ?1, settled_at = ?2
         WHERE request_id = ?3 AND status = 'reserved'",
        params![status, now as i64, reservation.request_id],
    )?;
    if changed != 1 {
        return Err(AppError::Other("cloud spend reservation was already settled or missing".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn reservation_survives_metrics_reset_and_blocks_the_cap() {
        let mut conn = setup();
        let reservation = reserve(&mut conn, "one", "0.0001", 10, 4, 86_400).unwrap();
        assert!(reservation.reserved_microusd > 0);
        assert!(reserve(&mut conn, "two", "0.0001", 10, 4, 86_401).is_err());
    }

    #[test]
    fn completion_releases_unused_output_reservation() {
        let mut conn = setup();
        let reservation = reserve(&mut conn, "one", "0.0002", 10, 10, 86_400).unwrap();
        complete(&conn, reservation, 1, 86_401).unwrap();
        assert!(reserve(&mut conn, "two", "0.0002", 10, 6, 86_402).is_ok());
    }

    #[test]
    fn cancellation_keeps_the_conservative_reservation() {
        let mut conn = setup();
        let reservation = reserve(&mut conn, "one", "0.0001", 10, 4, 86_400).unwrap();
        retain_reservation(&conn, reservation, "cancelled", 86_401).unwrap();
        assert!(reserve(&mut conn, "two", "0.0001", 10, 4, 86_402).is_err());
    }
}
