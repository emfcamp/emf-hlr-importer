use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write, self};
use std::net::TcpStream;
use std::collections::HashMap;
use serde::Deserialize;

static DEFAULTS_PATH: &str = "./defaults.csv";
static OSMO_HLR_ADDRESS: &str = "127.0.0.1:4258";

#[derive(Deserialize, Debug, Clone)]
struct DefaultsRecord {
    #[serde(rename = "IMSI")]
    imsi: String,
    #[serde(rename = "DefaultMSISDN")]
    default_msisdn: u64,
}

#[derive(Deserialize, Debug, Clone)]
struct KeysRecord {
    #[serde(rename = "IMSI")]
    imsi: String,
    #[serde(rename = "KI")]
    ki: String,
    #[serde(rename = "OPC")]
    opc: String,
}

/// Read the default MSISDN file.
/// Returns a mapping from IMSI to default MSISDN, and the latest default MSISDN observed.
fn read_defaults(f: &mut File) -> io::Result<(HashMap<String, u64>, u64)> {
    let mut reader = csv::Reader::from_reader(f);
    let mut imsis = HashMap::new();
    let mut biggest = 904_00000;
    for res in reader.deserialize() {
        let record: DefaultsRecord = match res {
            Ok(v) => v,
            Err(e) => {
                eprintln!("fatal: could not deserialize record in defaults file!");
                eprintln!("       error: {e}");
                std::process::exit(3);
            }
        };
        imsis.insert(record.imsi, record.default_msisdn);
        biggest = std::cmp::max(biggest, record.default_msisdn);
    }
    Ok((imsis, biggest))
}

fn open_csv(path: &str) -> io::Result<csv::Reader<File>> {
    let file = File::open(path)?;
    Ok(csv::Reader::from_reader(file))
}

fn main() {
    let mut args = std::env::args();
    let our_bin = args.next().unwrap();
    let Some(first_file) = args.next() else {
        eprintln!("usage: {our_bin} file_to_import.csv [additional_files...]");
        std::process::exit(1);
    };
    let mut all_files = args.collect::<Vec<_>>();
    all_files.insert(0, first_file);

    println!("[+] using default msisdn csv at {DEFAULTS_PATH}");
    let mut defaults = OpenOptions::new()
        .read(true)
        .append(true)
        .open(DEFAULTS_PATH)
        .unwrap();
    let (imsis, mut latest_default) = read_defaults(&mut defaults).unwrap();
    defaults.seek(SeekFrom::End(0)).expect("seek failed");
    println!("[+] {} MSISDNs in database; last was {latest_default}", imsis.len());

    println!("[+] connecting to HLR");
    let tcp_hlr = TcpStream::connect(OSMO_HLR_ADDRESS).unwrap();
    let tcp_hlr_clone = tcp_hlr.try_clone().unwrap();
    let mut hlr = rexpect::session::spawn_stream(tcp_hlr, tcp_hlr_clone, Some(1500));
    hlr.exp_string("OsmoHLR> ").unwrap();
    hlr.send_line("enable").unwrap();
    hlr.exp_string("OsmoHLR# ").unwrap();

    println!("[+] importing {} files", all_files.len());

    let mut cnt_new_default = 0;
    let mut cnt_new_hlr = 0;

    for file in all_files {
        println!("[+] importing {file}...");
        let mut reader = open_csv(&file).unwrap();
        for res in reader.deserialize() {
            let record: KeysRecord = match res {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("fatal: could not deserialize record");
                    eprintln!("       in file {file}");
                    eprintln!("       error: {e}");
                    std::process::exit(2);
                }
            };

            let imsi = record.imsi;

            let default_msisdn = if let Some(d) = imsis.get(&imsi) {
                // Don't make a new default MSISDN if we already have one.
                *d
            } else {
                // Make a new default MSISDN by incrementing the biggest one
                // we've seen so far.
                latest_default += 1;
                assert!(latest_default < 90500000);
                let ret = latest_default;

                write!(defaults, "{imsi},{ret}\n")
                    .expect("failed to write to defaults file!");
                
                cnt_new_default += 1;

                ret
            };

            hlr.send_line(&format!("subscriber imsi {imsi} show")).unwrap();
            let show_res = hlr.exp_string("OsmoHLR# ").unwrap();
            let show_res_first = show_res.lines().nth(1).unwrap();

            if !show_res_first.starts_with("% No subscriber") {
                // Already did this one!
                continue;
            }

            let mut expect_result = |line: String, wanted: &str| {
                hlr.send_line(&line).unwrap();
                let full_res = hlr.exp_string("OsmoHLR# ").unwrap();
                // The first line is what we echoed back, so we need to strip it
                let first_newline = full_res.find('\n').unwrap();
                let res = &full_res[first_newline+1..];
                if !res.starts_with(wanted) || (wanted.is_empty() && !res.is_empty()) {
                    eprintln!("fatal: weird HLR response for {imsi}");
                    eprintln!("{full_res}");
                    std::process::exit(4);
                }
            };

            expect_result(format!("subscriber imsi {imsi} create"), "% Created subscriber");
            expect_result(format!("subscriber imsi {imsi} update msisdn {default_msisdn}"), "% Updated subscriber");
            expect_result(format!("subscriber imsi {imsi} update aud3g milenage k {} opc {}", record.ki, record.opc), "");
            cnt_new_hlr += 1;
        }
    }

    defaults.sync_all().unwrap();

    println!("[+] {cnt_new_default} new defaults, {cnt_new_hlr} added to HLR");
}
