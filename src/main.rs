use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;

const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";

struct LineCoverage {
    da_hits: Option<u64>,
    // None means '-' (block never reached); Some(0) means reachable but not taken
    branches: Vec<Option<u64>>,
}

impl LineCoverage {
    fn color(&self) -> &'static str {
        let any_hit = self.da_hits.map_or(false, |h| h > 0)
            || self.branches.iter().any(|b| matches!(b, Some(h) if *h > 0));
        let any_missed = self.da_hits.map_or(false, |h| h == 0)
            || self.branches.iter().any(|b| !matches!(b, Some(h) if *h > 0));

        if !any_missed {
            GREEN
        } else if any_hit {
            YELLOW
        } else {
            RED
        }
    }

    // Text shown in the hits column. Lines with branch data show each branch
    // count separated by '/' (e.g. "4/-" or "1/3") so partial coverage is
    // immediately readable. Lines with only a DA record show the raw count.
    fn hits_display(&self) -> String {
        if !self.branches.is_empty() {
            self.branches
                .iter()
                .map(|b| b.map_or_else(|| "-".to_string(), |h| h.to_string()))
                .collect::<Vec<_>>()
                .join("/")
        } else {
            self.da_hits.unwrap_or(0).to_string()
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
    let hits_width = coverage
        .values()
        .map(|c| c.hits_display().len())
        .max()
        .unwrap_or(1);

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        match coverage.get(&line_num) {
            Some(cov) => {
                let gutter = format!(
                    "{:>lw$}  {:>hw$}  \u{2502}  ",
                    line_num,
                    cov.hits_display(),
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
                // DA format: <line>,<hits>[,<checksum>]
                if let Some((line_str, rest)) = data.split_once(',') {
                    let hits_str = rest.split(',').next().unwrap_or(rest);
                    if let (Ok(line_num), Ok(hits)) =
                        (line_str.parse::<usize>(), hits_str.parse::<u64>())
                    {
                        if line_num > 0 {
                            coverage
                                .entry(line_num)
                                .and_modify(|c| c.da_hits = Some(hits))
                                .or_insert(LineCoverage { da_hits: Some(hits), branches: vec![] });
                        }
                    }
                }
            } else if let Some(data) = line.strip_prefix("BRDA:") {
                // BRDA format: <line>,<block>,<branch>,<taken>
                // taken is '-' if the containing block was never executed
                let parts: Vec<&str> = data.splitn(4, ',').collect();
                if parts.len() == 4 {
                    if let Ok(line_num) = parts[0].parse::<usize>() {
                        if line_num > 0 {
                            let taken = if parts[3] == "-" {
                                None
                            } else {
                                parts[3].parse::<u64>().ok()
                            };
                            coverage
                                .entry(line_num)
                                .and_modify(|c| c.branches.push(taken))
                                .or_insert(LineCoverage { da_hits: None, branches: vec![taken] });
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

    if sf_path.ends_with(source_path) || source_path.ends_with(sf_path) {
        return true;
    }

    false
}
