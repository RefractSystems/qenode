use anyhow::{anyhow, Context, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use clap::Parser;
use object::{Object, ObjectSection, ObjectSymbol, SymbolKind};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Analyze Guest Firmware Coverage (Rust version)"
)]
struct Args {
    /// Path to .drcov file
    drcov: PathBuf,

    /// Path to ELF firmware file
    elf: PathBuf,

    /// Fail if total coverage is below this percentage
    #[arg(long)]
    fail_under: Option<f64>,

    /// Print all functions
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Interval {
    start: u64,
    end: u64,
}

struct SymbolInfo {
    name: String,
    address: u64,
    size: u64,
}

fn parse_drcov(path: &PathBuf) -> Result<Vec<Interval>> {
    let content =
        fs::read(path).with_context(|| format!("Failed to read drcov file: {:?}", path))?;

    let marker = b"BB Table: ";
    let idx = content
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| anyhow!("Could not find BB Table in drcov file"))?;

    let line_end = content[idx..]
        .iter()
        .position(|&b| b == b'\n')
        .ok_or_else(|| anyhow!("Malformed drcov file: missing newline after BB Table"))?;

    let count_str = std::str::from_utf8(&content[idx + marker.len()..idx + line_end])?
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("Could not parse BB count"))?;

    let count: usize = count_str.parse()?;
    let data = &content[idx + line_end + 1..];
    let mut bbs = Vec::with_capacity(count);

    // bb_entry_t: uint32 start, uint16 size, uint16 mod_id (8 bytes total)
    let entry_size = 8;
    for i in 0..count {
        let offset = i * entry_size;
        if offset + entry_size > data.len() {
            break;
        }
        let mut reader = Cursor::new(&data[offset..offset + entry_size]);
        let start = reader.read_u32::<LittleEndian>()? as u64;
        let size = reader.read_u16::<LittleEndian>()? as u64;
        // Skip mod_id
        bbs.push(Interval {
            start,
            end: start + size,
        });
    }

    Ok(bbs)
}

fn merge_intervals(mut intervals: Vec<Interval>) -> Vec<Interval> {
    if intervals.is_empty() {
        return vec![];
    }
    intervals.sort();

    let mut merged = Vec::new();
    let mut curr = intervals[0].clone();

    for next in intervals.into_iter().skip(1) {
        if next.start <= curr.end {
            curr.end = curr.end.max(next.end);
        } else {
            merged.push(curr);
            curr = next;
        }
    }
    merged.push(curr);
    merged
}

fn get_elf_symbols(path: &PathBuf) -> Result<Vec<SymbolInfo>> {
    let data = fs::read(path).with_context(|| format!("Failed to read ELF file: {:?}", path))?;
    let obj = object::File::parse(&*data).context("Failed to parse ELF file")?;

    let mut symbols = Vec::new();
    for symbol in obj.symbols() {
        // Only include symbols in sections (skip absolute symbols)
        let section_index = match symbol.section() {
            object::SymbolSection::Section(idx) => idx,
            _ => continue,
        };

        // Check if the section is executable (usually where code lives)
        let section = obj.section_by_index(section_index)?;
        if section.kind() != object::SectionKind::Text {
            continue;
        }

        if (symbol.kind() == SymbolKind::Text || symbol.kind() == SymbolKind::Unknown)
            && symbol.address() != 0
        {
            if let Ok(name) = symbol.name() {
                if !name.is_empty() && !name.starts_with('$') {
                    symbols.push(SymbolInfo {
                        name: name.to_string(),
                        address: symbol.address(),
                        size: symbol.size(),
                    });
                }
            }
        }
    }

    symbols.sort_by_key(|s| s.address);

    // Refine sizes if they are 0
    for i in 0..symbols.len().saturating_sub(1) {
        if symbols[i].size == 0 {
            symbols[i].size = symbols[i + 1].address - symbols[i].address;
        }
    }

    if let Some(last) = symbols.last_mut() {
        if last.size == 0 {
            last.size = 16; // Fallback
        }
    }

    Ok(symbols)
}

fn calculate_coverage(sym_start: u64, sym_end: u64, executed_intervals: &[Interval]) -> u64 {
    let mut exec_bytes = 0;

    // Binary search for relevant intervals
    let idx_start = match executed_intervals.binary_search_by(|i| {
        if i.end <= sym_start {
            std::cmp::Ordering::Less
        } else if i.start >= sym_end {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(idx) => {
            // Find the first one that might overlap
            let mut start = idx;
            while start > 0 && executed_intervals[start - 1].end > sym_start {
                start -= 1;
            }
            start
        }
        Err(idx) => idx,
    };

    for interval in &executed_intervals[idx_start..] {
        if interval.start >= sym_end {
            break;
        }

        let intersect_start = sym_start.max(interval.start);
        let intersect_end = sym_end.min(interval.end);

        if intersect_end > intersect_start {
            exec_bytes += intersect_end - intersect_start;
        }
    }

    exec_bytes
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_ansi(true)
        .init();

    let args = Args::parse();

    let bb_intervals = parse_drcov(&args.drcov)?;
    if bb_intervals.is_empty() {
        info!("No execution data found.");
        std::process::exit(1);
    }

    let executed_intervals = merge_intervals(bb_intervals);

    let symbols = get_elf_symbols(&args.elf)?;
    if symbols.is_empty() {
        info!("No symbols found to analyze.");
        std::process::exit(1);
    }

    info!("Coverage Report for {:?}", args.elf);
    info!(
        "BBs: {}, Functions: {}",
        executed_intervals.len(),
        symbols.len()
    );
    info!("{:-<60}", "");
    info!(
        "{:<30} {:<10} {:<10}",
        "Function Name", "Executed?", "Coverage"
    );
    info!("{:-<60}", "");

    let mut total_func_size = 0;
    let mut total_exec_size = 0;

    for sym in symbols {
        if sym.size == 0 {
            continue;
        }

        let exec_count =
            calculate_coverage(sym.address, sym.address + sym.size, &executed_intervals);
        let coverage = (exec_count as f64 / sym.size as f64) * 100.0;
        let executed = if exec_count > 0 { "Yes" } else { "No" };

        if args.verbose || exec_count > 0 || coverage < 100.0 {
            info!("{:<30} {:<10} {:>8.1}%", sym.name, executed, coverage);
        }

        total_func_size += sym.size;
        total_exec_size += exec_count;
    }

    info!("{:-<60}", "");
    let total_coverage = if total_func_size > 0 {
        (total_exec_size as f64 / total_func_size as f64) * 100.0
    } else {
        0.0
    };
    info!("{:<30} {:<10} {:>8.1}%", "TOTAL", "", total_coverage);
    info!("{:-<60}", "");

    if let Some(fail_under) = args.fail_under {
        if total_coverage < fail_under {
            error!(
                "FAILED: Coverage {:.1}% is below required {:.1}%",
                total_coverage, fail_under
            );
            std::process::exit(1);
        }
    }

    info!("Coverage check passed.");
    Ok(())
}
