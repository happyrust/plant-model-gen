use aios_core::RefU64;
use parse_pdms_db::parse::{parse_ele_data_with_info_sync, parse_file_db_basic_data};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: cargo run --example inspect_pdms_refno_attr -- <db-file> <ref0/ref1>"
    );
    let path = PathBuf::from(&args[1]);
    let refno = args[2]
        .parse::<RefU64>()
        .map_err(|e| anyhow::anyhow!("invalid refno {}: {:?}", args[2], e))?;
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let basic = parse_file_db_basic_data(&path, &file_name, "inspect")?;
    let Some(entry) = basic.refno_table_map.get(&refno) else {
        anyhow::bail!("refno not found in index: {refno}");
    };
    let pos = entry.pos;
    drop(entry);
    let db_info = aios_core::get_default_pdms_db_info();
    let ele = parse_ele_data_with_info_sync(&basic.bytes[pos - 4..], &db_info)?;
    let att = ele.whole_attmap.merge();
    println!(
        "refno={} noun={} owner={} children={:?}",
        refno,
        att.get_type_str(),
        ele.owner,
        ele.children
    );
    for key in [
        "DIAM", "HEIG", "RADI", "DTOP", "DBOT", "XBOT", "YBOT", "XTOP", "YTOP", "XOFF", "YOFF",
        "DHEI", "SHEI", "DRAD", "SDIA", "SWID", "STHI", "POS", "ORI", "ORRF", "DDPR", "DDDF",
        "DKEY", "DESP", "PKDI", "ZDIS", "PKEY", "PAXI", "POSL", "YDIR", "OPDI", "BANG", "SCTN",
        "CATR", "SPRE", "LSTU", "HSTU", "DRNS", "DRNE",
    ] {
        if let Some(value) = att.map.get(key) {
            println!("{key} = {:?}", value);
        }
    }
    println!("keys={:?}", att.map.keys().cloned().collect::<Vec<_>>());
    Ok(())
}
