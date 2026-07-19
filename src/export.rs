//! Records export — bundle your local utility data into a single .zip.
//!
//! Everything utiman keeps locally (balance history + the archived usage/bill
//! series) is written out as CSVs and packed into one store-only ZIP. We build
//! the ZIP by hand (a few dozen bytes of header per file + a CRC32) rather than
//! pull in a zip crate: the data is small and text, and PDFs — the only reason
//! to want real compression — are already compressed. The bytes it produces
//! open in Finder, `unzip`, and every spreadsheet app.

use crate::manifest::load_providers;

const README: &str = "\
utiman export
=============
balances.csv        Every recorded balance snapshot, across all providers.
series/*.csv         One file per archived series (bill amounts, usage, ...).

Columns in balances.csv: provider, date (UTC), unix_ts, balance, due_date.
Columns in each series file: label, value.
All data came from your own provider CLIs and never left this machine.
";

/// Build the export ZIP in memory and return its bytes.
pub fn build_bundle() -> Vec<u8> {
    let mut zip = Zip::new();
    zip.add("README.txt", README.as_bytes());
    zip.add("balances.csv", balances_csv().as_bytes());
    for (name, csv) in series_csvs() {
        zip.add(&format!("series/{name}.csv"), csv.as_bytes());
    }
    zip.finish()
}

/// All providers' balance snapshots as one CSV.
fn balances_csv() -> String {
    let mut out = String::from("provider,date,unix_ts,balance,due_date\n");
    for p in load_providers() {
        for s in crate::snapshots::read(&p.manifest.id) {
            out.push_str(&balance_row(&p.manifest.id, &s));
        }
    }
    out
}

/// One `balances.csv` data row (pure — the composition/formatting under test).
fn balance_row(id: &str, s: &crate::snapshots::Snapshot) -> String {
    let d = crate::dates::from_unix(s.ts as i64);
    format!(
        "{},{:04}-{:02}-{:02},{},{},{}\n",
        field(id),
        d.year,
        d.month,
        d.day,
        s.ts,
        s.balance,
        field(s.due_date.as_deref().unwrap_or("")),
    )
}

/// One (filename-stem, CSV) per archived series file on disk.
fn series_csvs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(crate::archive::dir()) else {
        return out;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".jsonl")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    for stem in names {
        // Files are named "<provider>__<series>.jsonl".
        let Some((prov, series)) = stem.split_once("__") else {
            continue;
        };
        let mut csv = String::from("label,value\n");
        for pt in crate::archive::read(prov, series) {
            csv.push_str(&format!("{},{}\n", field(&pt.label), pt.value));
        }
        out.push((stem, csv));
    }
    out
}

/// CSV-escape a field: quote it when it contains a comma, quote, or newline,
/// doubling any embedded quotes (RFC 4180).
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ---------- minimal store-only ZIP writer ----------

struct Entry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

struct Zip {
    buf: Vec<u8>,
    entries: Vec<Entry>,
}

impl Zip {
    fn new() -> Self {
        Zip {
            buf: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Append a stored (uncompressed) file.
    fn add(&mut self, name: &str, data: &[u8]) {
        let crc = crc32(data);
        let size = data.len() as u32;
        let offset = self.buf.len() as u32;
        // Local file header.
        self.buf.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // flags
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // method: store
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // mod time
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // mod date
        self.buf.extend_from_slice(&crc.to_le_bytes());
        self.buf.extend_from_slice(&size.to_le_bytes()); // compressed
        self.buf.extend_from_slice(&size.to_le_bytes()); // uncompressed
        self.buf
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra len
        self.buf.extend_from_slice(name.as_bytes());
        self.buf.extend_from_slice(data);
        self.entries.push(Entry {
            name: name.to_string(),
            crc,
            size,
            offset,
        });
    }

    /// Write the central directory + end record and return the ZIP bytes.
    fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.buf.len() as u32;
        for e in &self.entries {
            self.buf.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            self.buf.extend_from_slice(&20u16.to_le_bytes()); // version made by
            self.buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // flags
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // method: store
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // mod time
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // mod date
            self.buf.extend_from_slice(&e.crc.to_le_bytes());
            self.buf.extend_from_slice(&e.size.to_le_bytes());
            self.buf.extend_from_slice(&e.size.to_le_bytes());
            self.buf
                .extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // extra
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // comment
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // disk #
            self.buf.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            self.buf.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            self.buf.extend_from_slice(&e.offset.to_le_bytes());
            self.buf.extend_from_slice(e.name.as_bytes());
        }
        let cd_size = self.buf.len() as u32 - cd_offset;
        let n = self.entries.len() as u16;
        // End of central directory.
        self.buf.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // disk #
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        self.buf.extend_from_slice(&n.to_le_bytes());
        self.buf.extend_from_slice(&n.to_le_bytes());
        self.buf.extend_from_slice(&cd_size.to_le_bytes());
        self.buf.extend_from_slice(&cd_offset.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes()); // comment len
        self.buf
    }
}

/// CRC-32 (IEEE, the polynomial ZIP uses), bit-by-bit — no lookup table needed
/// for the small volumes here.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vector() {
        // The canonical CRC-32 of "123456789" is 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn field_escapes_only_when_needed() {
        assert_eq!(field("electric"), "electric");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn balance_row_formats_date_money_and_escapes() {
        use crate::snapshots::Snapshot;
        // ts 0 = 1970-01-01 UTC.
        let plain = balance_row(
            "fpl",
            &Snapshot {
                ts: 0,
                balance: 42.5,
                due_date: Some("8/5/2026".into()),
            },
        );
        assert_eq!(plain, "fpl,1970-01-01,0,42.5,8/5/2026\n");
        // A due date with a comma is CSV-quoted; a missing one is empty.
        let comma = balance_row(
            "x",
            &Snapshot {
                ts: 0,
                balance: -3.0,
                due_date: Some("July 11, 2026".into()),
            },
        );
        assert_eq!(comma, "x,1970-01-01,0,-3,\"July 11, 2026\"\n");
    }

    #[test]
    fn zip_has_signature_and_eocd() {
        let mut z = Zip::new();
        z.add("a.txt", b"hello");
        let bytes = z.finish();
        assert_eq!(&bytes[0..4], &0x0403_4b50u32.to_le_bytes()); // local header
                                                                 // End-of-central-directory signature appears near the tail.
        assert!(bytes.windows(4).any(|w| w == 0x0605_4b50u32.to_le_bytes()));
    }
}
