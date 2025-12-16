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
    let baseline_dir = manifest_dir.join("tests/baseline");

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
        // Return Ok to skip silently if baseline data is missing.
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
        if !golden.step_type.eq_ignore_ascii_case("dance-single") {
            continue;
        }
        let key = (
            golden.step_type.to_ascii_lowercase(),
            golden.difficulty.to_ascii_lowercase(),
        );
        golden_map.entry(key).or_default().push(golden.hash);
    }

    let mut rssp_map: HashMap<(String, String), Vec<String>> = HashMap::new();
    for chart in rssp_charts {
        if !chart.step_type.eq_ignore_ascii_case("dance-single") {
            continue;
        }
        let key = (
            chart.step_type.to_ascii_lowercase(),
            chart.difficulty.to_ascii_lowercase(),
        );
        rssp_map.entry(key).or_default().push(chart.hash);
    }

    for ((step_type, difficulty), expected_hashes) in golden_map {
        let Some(actual_hashes) = rssp_map.remove(&(step_type.clone(), difficulty.clone())) else {
            return Err(Failed::from(format!(
                "\n\nMISSING CHART DETECTED\nFile: {}\nExpected: {} {}\n",
                path.display(),
                step_type,
                difficulty
            )));
        };

        // For non-edit charts, only compare the first occurrence to handle duplicates gracefully.
        if !difficulty.eq_ignore_ascii_case("edit") {
            let golden_hash = expected_hashes.first().unwrap();
            let actual_hash = actual_hashes.first().unwrap();
            if golden_hash != actual_hash {
                return Err(Failed::from(format!(
                    "\n\nMISMATCH DETECTED\nFile: {}\nChart: {} {}\nRSSP Hash:   {}\nGolden Hash: {}\n",
                    path.display(),
                    step_type,
                    difficulty,
                    actual_hash,
                    golden_hash
                )));
            }
            continue;
        }

        // Edits can legitimately have multiple charts; compare multisets.
        let mut expected_sorted = expected_hashes.clone();
        let mut actual_sorted = actual_hashes.clone();
        expected_sorted.sort();
        actual_sorted.sort();

        if expected_sorted != actual_sorted {
            return Err(Failed::from(format!(
                "\n\nMISMATCH DETECTED\nFile: {}\nChart: {} {}\nRSSP Hashes:   {:?}\nGolden Hashes: {:?}\n",
                path.display(),
                step_type,
                difficulty,
                actual_sorted,
                expected_sorted
            )));
        }
    }

    Ok(())
}
