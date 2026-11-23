//! Read-only check of a QRZ logbook key against the live API: the
//! logbook's status, then a count of what a fetch returns. Sends only
//! `STATUS` and `FETCH`; never inserts or deletes.
//!
//! ```text
//! cargo run -p qrz-sync --example readonly_check -- <key-file> [YYYY-MM-DD]
//! ```
//!
//! With a date, fetches records modified since then; without, the whole
//! logbook.

use chrono::NaiveDate;
use qrz_sync::{QrzClient, QrzConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let key_file = args.next().ok_or("usage: readonly_check <key-file> [YYYY-MM-DD]")?;
    let since = args
        .next()
        .map(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d"))
        .transpose()?;
    let key = std::fs::read_to_string(key_file)?;
    let client = QrzClient::new(QrzConfig::new(key.trim()));

    let status = client.status().await?;
    println!("logbook: {status:?}");

    let started = std::time::Instant::now();
    let report = client.fetch_modified_since(since).await?;
    let confirmed = report.records.iter().filter(|r| r.confirmed).count();
    let without_logid = report.records.iter().filter(|r| r.logid.is_none()).count();
    let mut ids: Vec<u64> = report.records.iter().filter_map(|r| r.logid).collect();
    ids.sort_unstable();
    ids.dedup();
    println!(
        "fetched {} records in {:.1}s: {confirmed} confirmed, {} unreadable, \
         {without_logid} without a logid, {} distinct logids",
        report.records.len(),
        started.elapsed().as_secs_f32(),
        report.unreadable,
        ids.len(),
    );
    Ok(())
}
