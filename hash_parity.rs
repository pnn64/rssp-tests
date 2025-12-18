use std::fs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use rssp; 
use serde::Deserialize;
use walkdir::WalkDir;
use libtest_mimic::{Arguments, Trial, Failed};

#[derive(Debug, Deserialize)]
struct GoldenChart {
    difficulty: String,
    #[serde(rename = "steps_type")]
    step_type: String,
    hash: String,
}

fn main() {
    let args = Arguments::from_args();

    // 1. Setup paths
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Assuming the submodule is mounted at 'tests'
    let packs_dir = manifest_dir.join("tests/packs");
    let baseline_dir = resolve_baseline_dir(manifest_dir.join("tests/baseline"));

    if !packs_dir.exists() {
        println!("No tests/packs directory found.");
        return;
    }

    // 2. Collect all test cases
    let mut tests = Vec::new();

    for entry in WalkDir::new(&packs_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Check for .zst extension
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "zst" {
            continue;
        }

        // Check the "inner" extension (e.g. "file.sm.zst" -> "sm")
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let inner_path = Path::new(stem);
        let inner_extension = inner_path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        if inner_extension != "sm" && inner_extension != "ssc" {
            continue;
        }

        // Create a pretty name: "PackName/SongName/file.ssc.zst"
        let test_name = path.strip_prefix(&packs_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let path_clone = path.to_path_buf();
        let baseline_dir_clone = baseline_dir.clone();
        let extension_clone = inner_extension.clone();

        // 3. Create a Trial
        let trial = Trial::test(test_name, move || {
            check_file(&path_clone, &extension_clone, &baseline_dir_clone)
        });

        tests.push(trial);
    }

    // 4. Run tests
    libtest_mimic::run(&args, tests).exit();
}

fn resolve_baseline_dir(default_dir: PathBuf) -> PathBuf {
    let nested = default_dir.join("baseline");
    if nested.exists() {
        return nested;
    }
    default_dir
}

fn check_file(path: &Path, extension: &str, baseline_dir: &Path) -> Result<(), Failed> {
    // 1. Read Compressed Simfile
    let compressed_bytes = fs::read(path)
        .map_err(|e| format!("Failed to read file: {}", e))?;
    
    // 2. Decompress Simfile
    let raw_bytes = zstd::decode_all(&compressed_bytes[..])
        .map_err(|e| format!("Failed to decompress simfile: {}", e))?;
    
    // 3. Compute Hash (on raw bytes) to find Baseline JSON
    let file_hash = format!("{:x}", md5::compute(&raw_bytes));
    
    // Determine sharded subfolder (first 2 chars of hash)
    let subfolder = &file_hash[0..2];

    // Look for baseline/{xx}/{hash}.json.zst
    let golden_path = baseline_dir
        .join(subfolder)
        .join(format!("{}.json.zst", file_hash));

    if !golden_path.exists() {
        println!(
            "File: {} (no baseline found for hash {})",
            path.display(),
            file_hash
        );
        return Ok(()); 
    }

    // 4. Read & Decompress Golden JSON
    let compressed_golden = fs::read(&golden_path)
        .map_err(|e| format!("Failed to read baseline file: {}", e))?;
    
    let json_bytes = zstd::decode_all(&compressed_golden[..])
        .map_err(|e| format!("Failed to decompress baseline json: {}", e))?;

    let golden_charts: Vec<GoldenChart> = serde_json::from_slice(&json_bytes)
        .map_err(|e| format!("Failed to parse baseline JSON: {}", e))?;

    // 5. Run RSSP FAST Hashing (using decompressed raw_bytes)
    let rssp_charts = rssp::compute_all_hashes(&raw_bytes, extension)
        .map_err(|e| format!("RSSP Parsing Error: {}", e))?;

    // 6. Compare Charts (support multiple edits per difficulty)
    let mut golden_map: HashMap<(String, String), Vec<String>> = HashMap::new();
    for golden in golden_charts {
        let step_type_lower = golden.step_type.to_ascii_lowercase();
        if step_type_lower != "dance-single" && step_type_lower != "dance-double" {
            continue;
        }
        let key = (
            step_type_lower,
            golden.difficulty.to_ascii_lowercase(),
        );
        golden_map.entry(key).or_default().push(golden.hash);
    }

    let mut rssp_map: HashMap<(String, String), Vec<String>> = HashMap::new();
    for chart in rssp_charts {
        let step_type_lower = chart.step_type.to_ascii_lowercase();
        if step_type_lower != "dance-single" && step_type_lower != "dance-double" {
            continue;
        }
        let key = (
            step_type_lower,
            chart.difficulty.to_ascii_lowercase(),
        );
        rssp_map.entry(key).or_default().push(chart.hash);
    }

    let mut golden_entries: Vec<_> = golden_map.into_iter().collect();
    golden_entries.sort_by(|a, b| a.0.cmp(&b.0));

    println!("File: {}", path.display());

    for ((step_type, difficulty), expected_hashes) in golden_entries {
        let Some(actual_hashes) = rssp_map.remove(&(step_type.clone(), difficulty.clone())) else {
            println!(
                "  {} {}: baseline present, RSSP missing chart",
                step_type, difficulty
            );
            return Err(Failed::from(format!(
                "\n\nMISSING CHART DETECTED\nFile: {}\nExpected: {} {}\n",
                path.display(),
                step_type,
                difficulty
            )));
        };

        let count = expected_hashes.len().max(actual_hashes.len());
        for idx in 0..count {
            let expected = expected_hashes.get(idx);
            let actual = actual_hashes.get(idx);
            let status = if expected.is_some() && expected == actual {
                "... ok"
            } else {
                "... MISMATCH"
            };

            println!(
                "  {} {} [{}]: baseline: {} -> rssp: {} {}",
                step_type,
                difficulty,
                idx + 1,
                expected.map(|s| s.as_str()).unwrap_or("-"),
                actual.map(|s| s.as_str()).unwrap_or("-"),
                status
            );
        }

        if expected_hashes != actual_hashes {
            return Err(Failed::from(format!(
                "\n\nMISMATCH DETECTED\nFile: {}\nChart: {} {}\nRSSP Hashes:   {:?}\nGolden Hashes: {:?}\n",
                path.display(),
                step_type,
                difficulty,
                actual_hashes,
                expected_hashes
            )));
        }
        continue;
    }

    Ok(())
}
