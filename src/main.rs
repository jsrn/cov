use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;

const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";

// Coverage for a single line: tracks the min and max hits across all DA entries.
// A line with a ternary/inline branch may appear multiple times in the LCOV file.
struct LineCoverage {
    min: u64,
    max: u64,
}

impl LineCoverage {
    fn new(hits: u64) -> Self {
        Self { min: hits, max: hits }
    }

    fn update(&mut self, hits: u64) {
        self.min = self.min.min(hits);
        self.max = self.max.max(hits);
    }

    fn color(&self) -> &'static str {
        if self.max == 0 {
            RED    // never executed
        } else if self.min == 0 {
            YELLOW // partially covered (some branches missed)
        } else {
            GREEN  // fully covered
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <source_file>", args[0]);
        std::process::exit(1);
    }

    let source_path = &args[1];
    let lcov_path = "coverage/lcov/gitlab.lcov";

    let coverage = parse_lcov(lcov_path, source_path);

    let source = fs::read_to_string(source_path).unwrap_or_else(|e| {
        eprintln!("Error reading source file '{}': {}", source_path, e);
        std::process::exit(1);
    });

    let lines: Vec<&str> = source.lines().collect();
    let line_num_width = lines.len().to_string().len();
    let max_hits = coverage.values().map(|c| c.max).max().unwrap_or(0);
    let hits_width = max_hits.to_string().len().max(1);

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        match coverage.get(&line_num) {
            Some(cov) => {
                let gutter = format!(
                    "{:>lw$}  {:>hw$}  \u{2502}  ",
                    line_num,
                    cov.max,
                    lw = line_num_width,
                    hw = hits_width
                );
                println!("{}{}{}{}", cov.color(), gutter, line, RESET);
            }
            None => {
                let gutter = format!(
                    "{:>lw$}  {:>hw$}  \u{2502}  ",
                    line_num,
                    "",
                    lw = line_num_width,
                    hw = hits_width
                );
                println!("{}{}", gutter, line);
            }
        }
    }
}

fn parse_lcov(lcov_path: &str, source_path: &str) -> HashMap<usize, LineCoverage> {
    let source_canonical = Path::new(source_path).canonicalize().ok();

    let file = fs::File::open(lcov_path).unwrap_or_else(|e| {
        eprintln!("Error opening LCOV file '{}': {}", lcov_path, e);
        std::process::exit(1);
    });

    let reader = io::BufReader::new(file);
    let mut coverage: HashMap<usize, LineCoverage> = HashMap::new();
    let mut current_file_matches = false;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if let Some(sf_path) = line.strip_prefix("SF:") {
            current_file_matches =
                matches_source(sf_path, source_path, source_canonical.as_deref());
        } else if line == "end_of_record" {
            if current_file_matches {
                break;
            }
            current_file_matches = false;
        } else if current_file_matches {
            if let Some(data) = line.strip_prefix("DA:") {
                if let Some((line_str, rest)) = data.split_once(',') {
                    // DA format: <line>,<hits>[,<checksum>] — ignore optional checksum
                    let hits_str = rest.split(',').next().unwrap_or(rest);
                    if let (Ok(line_num), Ok(hits)) =
                        (line_str.parse::<usize>(), hits_str.parse::<u64>())
                    {
                        if line_num > 0 {
                            coverage
                                .entry(line_num)
                                .and_modify(|c| c.update(hits))
                                .or_insert_with(|| LineCoverage::new(hits));
                        }
                    }
                }
            } else if let Some(data) = line.strip_prefix("BRDA:") {
                // BRDA format: <line>,<block>,<branch>,<taken>
                // <taken> is '-' if the block was never reached, or a hit count
                let parts: Vec<&str> = data.splitn(4, ',').collect();
                if parts.len() == 4 {
                    if let Ok(line_num) = parts[0].parse::<usize>() {
                        if line_num > 0 {
                            let hits = if parts[3] == "-" { 0 } else { parts[3].parse::<u64>().unwrap_or(0) };
                            coverage
                                .entry(line_num)
                                .and_modify(|c| c.update(hits))
                                .or_insert_with(|| LineCoverage::new(hits));
                        }
                    }
                }
            }
        }
    }

    if coverage.is_empty() {
        eprintln!("Warning: no coverage data found for '{}'", source_path);
        eprintln!("Make sure the path matches an SF: entry in the LCOV file.");
    }

    coverage
}

fn matches_source(sf_path: &str, source_path: &str, source_canonical: Option<&Path>) -> bool {
    if sf_path == source_path {
        return true;
    }

    if let Some(canonical) = source_canonical {
        if let Ok(sf_canonical) = Path::new(sf_path).canonicalize() {
            if sf_canonical == canonical {
                return true;
            }
        }
    }

    // Handle relative vs absolute: check if one is a suffix of the other
    if sf_path.ends_with(source_path) || source_path.ends_with(sf_path) {
        return true;
    }

    false
}
