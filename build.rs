use serde::Deserialize;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct TradingInput {
    pair: Vec<[String; 2]>,
}

#[derive(Deserialize)]
struct OrcaPoolConfig {
    pubkey: String,
    mint_a: String,
    mint_b: String,
}

#[derive(Deserialize)]
struct OrcaInput {
    list: Vec<OrcaPoolConfig>,
}

fn main() {
    // 1. Tell Cargo to rerun this script ONLY if graph.json changes
    println!("cargo:rerun-if-changed=graph.json");
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    let graph_json_fp = manifest_dir.join("target").join("graph.json");
    // 2. Read and parse the JSON file
    let mut file = match File::open(graph_json_fp) {
        Ok(x) => x,
        Err(_) => match File::open(manifest_dir.join("graph.json")) {
            Ok(y) => y,
            Err(_) => panic!("failed to open graph.json file"),
        },
    };
    let mut json_str = String::new();
    file.read_to_string(&mut json_str)
        .expect("Failed to read graph.json");

    // 3. Determine the output path in the build directory (OUT_DIR)
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // ── trading.json → trading_data.rs ──────────────────────────────────────
    println!("cargo:rerun-if-changed=trading.json");

    // ── orca.json → orca_data.rs ─────────────────────────────────────────────
    println!("cargo:rerun-if-changed=target/orca.json");
    {
        let decode = |s: &str, label: &str| -> [u8; 32] {
            let v = bs58::decode(s)
                .into_vec()
                .unwrap_or_else(|e| panic!("invalid base58 {label} {s:?}: {e}"));
            assert_eq!(v.len(), 32, "{label} {s:?} is not 32 bytes");
            v.try_into().unwrap()
        };

        let mut pools_code =
            String::from("pub static ORCA_POOLS: &[([u8; 32], [u8; 32], [u8; 32])] = &[");

        let orca_json_fp = manifest_dir.join("target").join("orca.json");
        if let Ok(mut f) = File::open(&orca_json_fp) {
            let mut orca_str = String::new();
            f.read_to_string(&mut orca_str)
                .expect("failed to read orca.json");
            let orca: OrcaInput =
                serde_json::from_str(&orca_str).expect("failed to parse orca.json");
            for pool in &orca.list {
                let pk = decode(&pool.pubkey, "pubkey");
                let ma = decode(&pool.mint_a, "mint_a");
                let mb = decode(&pool.mint_b, "mint_b");
                pools_code.push_str(&format!("    ({pk:?}, {ma:?}, {mb:?}),\n"));
            }
        }
        pools_code.push_str("];\n");
        File::create(out_dir.join("orca_data.rs"))
            .expect("failed to create orca_data.rs")
            .write_all(pools_code.as_bytes())
            .expect("failed to write orca_data.rs");
    }
    {
        let mut trading_str = String::new();
        {
            let mut trading_json_fp = manifest_dir.join("target").join("trading.json");
            let mut f = match File::open(&trading_json_fp) {
                Ok(f) => f,
                Err(_) => {
                    trading_json_fp = manifest_dir.join("trading.json");
                    match File::open(&trading_json_fp) {
                        Ok(x) => x,
                        Err(e) => panic!("failed to open trading.json file: {e}"),
                    }
                }
            };
            f.read_to_string(&mut trading_str)
                .expect("failed to read trading.json")
        };
        let trading: TradingInput =
            serde_json::from_str(&trading_str).expect("failed to parse trading.json");

        let mut pairs_code = String::from("pub static TRADING_PAIRS: &[([u8; 32], [u8; 32])] = &[");
        for [a, b] in &trading.pair {
            let a_bytes = bs58::decode(a)
                .into_vec()
                .unwrap_or_else(|e| panic!("invalid base58 in trading.json mint_a {a:?}: {e}"));
            let b_bytes = bs58::decode(b)
                .into_vec()
                .unwrap_or_else(|e| panic!("invalid base58 in trading.json mint_b {b:?}: {e}"));
            assert_eq!(a_bytes.len(), 32, "mint_a {a:?} is not 32 bytes");
            assert_eq!(b_bytes.len(), 32, "mint_b {b:?} is not 32 bytes");
            pairs_code.push_str(&format!("    ({:?}, {:?}),\n", a_bytes, b_bytes));
        }
        pairs_code.push_str("];\n");

        let trading_dest = Path::new(&out_dir).join("trading_data.rs");
        File::create(&trading_dest)
            .expect("failed to create trading_data.rs")
            .write_all(pairs_code.as_bytes())
            .expect("failed to write trading_data.rs");
    }
}
