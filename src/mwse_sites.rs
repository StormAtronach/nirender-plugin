//! The check of an MWSE.dll's patch sites against the ports, before the DLL goes to a test
//! install (Brief 22, 2026-11-10).
//!
//! MWSE gives the address of each patch site to its helpers as an operand of an instruction
//! (`push imm32`, `mov r32, imm32`, `mov [m], imm32`), and its site lists are aligned values in
//! its data. So the 32-bit operands of the built DLL that fall inside a function of the table
//! name the places that the DLL patches there. A rule for each place tells whether the port of
//! the function follows the patch; `tools/mwse_sites.baseline` holds the reviewed places that
//! are no patch of a port (calls of a port's entry, constants that are data).
//!
//! The rules, in this order; the first that applies decides:
//! 1. The function has no port (census only): the place is listed, never failed.
//! 2. ENTRY, the place is the function's entry: it fails unless the baseline has it.
//! 3. HEAD, inside the bytes that the detour steals: it passes only at the `call` or `jmp` of a
//!    32-bit target that the head compare lets change (`hook::head_matches`), when a port serves
//!    the site or a known patch covers it.
//! 4. BODY, past the stolen bytes: it passes when a known patch, a served site or a port that
//!    runs the whole engine function covers it; else it fails unless the baseline has it.
//!
//! A baseline line is the value, the instruction form and its count: a new form or a higher
//! count of a value fails until it is reviewed.
//!
//! Limits: an address that MWSE computes at run time is no operand (the census of the game
//! start sees it); a patch that MWSE.json turns off still fails here (the safe side); a vtable
//! override is seen only when MWSE passes the function as the expected old value.

use std::collections::BTreeMap;

use crate::gen::funcs::{Func, FUNCS};
use crate::ports;

/// How the DLL holds a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Form {
    /// `push imm32` (0x68).
    Push,
    /// `mov r32, imm32` (0xB8 to 0xBF).
    MovReg,
    /// `mov [m], imm32` (0xC7): the value ends the instruction.
    MovMem,
    /// An aligned value in a section that is not code: a site list.
    Data,
}

impl Form {
    pub fn name(self) -> &'static str {
        match self {
            Form::Push => "push",
            Form::MovReg => "mov-r32",
            Form::MovMem => "mov-m",
            Form::Data => "data",
        }
    }

    pub fn parse(text: &str) -> Option<Form> {
        [Form::Push, Form::MovReg, Form::MovMem, Form::Data].into_iter().find(|form| form.name() == text)
    }
}

/// The sections of a PE file: the name and the raw bytes.
pub fn sections(pe: &[u8]) -> Vec<(String, &[u8])> {
    let word = |at: usize| u16::from_le_bytes([pe[at], pe[at + 1]]) as usize;
    let dword = |at: usize| u32::from_le_bytes([pe[at], pe[at + 1], pe[at + 2], pe[at + 3]]) as usize;
    let header = dword(0x3C);
    let count = word(header + 6);
    let optional = word(header + 20);
    let table = header + 24 + optional;
    (0..count)
        .map(|index| {
            let entry = table + 40 * index;
            let name = String::from_utf8_lossy(&pe[entry..entry + 8]).trim_end_matches('\0').to_string();
            let (size, raw) = (dword(entry + 16), dword(entry + 20));
            (name, &pe[raw.min(pe.len())..(raw + size).min(pe.len())])
        })
        .collect()
}

/// The form of the 4 bytes at `at` of the code `text`, when they are the immediate operand of
/// one of the three instructions; else None.
fn code_form(text: &[u8], at: usize) -> Option<Form> {
    let before = |back: usize| if at >= back { Some(text[at - back]) } else { None };
    match before(1) {
        Some(0x68) => return Some(Form::Push),
        Some(0xB8..=0xBF) => return Some(Form::MovReg),
        _ => {}
    }
    // `mov [m], imm32`: C7, a ModR/M byte, at most a SIB byte and a byte of displacement.
    if (2..=4).any(|back| before(back) == Some(0xC7)) {
        return Some(Form::MovMem);
    }
    None
}

/// Each value of `pe` for which `inside` is true, by form, with the number of its occurrences.
pub fn operands(pe: &[u8], inside: impl Fn(u32) -> bool) -> BTreeMap<(u32, Form), usize> {
    let mut found = BTreeMap::new();
    for (name, bytes) in sections(pe) {
        let code = name == ".text";
        let step = if code { 1 } else { 4 };
        let mut at = 0;
        while at + 4 <= bytes.len() {
            let value = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            if inside(value) {
                let form = if code { code_form(bytes, at) } else { Some(Form::Data) };
                if let Some(form) = form {
                    *found.entry((value, form)).or_insert(0) += 1;
                }
            }
            at += step;
        }
    }
    found
}

/// Where a value lies in a function of the table, by the rules of the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    CensusOnly,
    Entry,
    Head,
    Body,
}

impl Place {
    fn name(self) -> &'static str {
        match self {
            Place::CensusOnly => "census only",
            Place::Entry => "ENTRY",
            Place::Head => "HEAD",
            Place::Body => "BODY",
        }
    }
}

/// The function of the table that holds `value`.
pub fn function_of(value: u32) -> Option<&'static Func> {
    let value = value as usize;
    let index = FUNCS.partition_point(|func| func.ea <= value).checked_sub(1)?;
    let func = &FUNCS[index];
    (value < func.ea + func.size).then_some(func)
}

/// The place of `value` in `func`, and the port's name when the function has one.
pub fn place(func: &Func, value: u32) -> (Place, Option<&'static str>) {
    let Some(port) = ports::all().find(|port| port.addr == func.ea) else {
        return (Place::CensusOnly, None);
    };
    let offset = value as usize - func.ea;
    let place = if offset == 0 {
        Place::Entry
    } else if offset < func.steal.max(5) {
        Place::Head
    } else {
        Place::Body
    };
    (place, Some(port.name))
}

/// True when a port follows a patch at `value`: a known patch covers it, or a port calls the
/// target that is in memory at that call site.
pub fn followed(value: u32) -> bool {
    ports::followed_range(value as usize) || ports::serves_site(value as usize)
}

/// True when the head compare lets the 32-bit target of the `call` or `jmp` at `value` change.
fn head_target(func: &Func, value: u32) -> bool {
    let opcode = value as usize - func.ea;
    func.relocs.iter().any(|&reloc| reloc != 0 && reloc as usize == opcode + 1)
}

/// A line of the baseline: the value, the form, the count, and why it is no patch of a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewed {
    pub value: u32,
    pub form: Form,
    pub count: usize,
}

/// The lines of the baseline; a line that does not parse is an error with its number.
pub fn parse_baseline(text: &str) -> Result<Vec<Reviewed>, String> {
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let parsed = (|| {
            let value = u32::from_str_radix(words.next()?.strip_prefix("0x")?, 16).ok()?;
            let form = Form::parse(words.next()?)?;
            let count = words.next()?.parse().ok()?;
            // The reason is not optional.
            words.next()?;
            Some(Reviewed { value, form, count })
        })();
        lines.push(parsed.ok_or_else(|| format!("line {}: not \"0x<value> <form> <count> <why>\"", index + 1))?);
    }
    Ok(lines)
}

/// The result of the check of one DLL.
#[derive(Debug, Default)]
pub struct Report {
    /// One line for each occurrence kind, in the order of the values.
    pub lines: Vec<String>,
    /// The lines of the failures.
    pub failures: Vec<String>,
    /// The baseline lines whose value and form the DLL does not have.
    pub stale: Vec<String>,
}

/// Checks the operands of `pe` by the rules. `followed` tells whether a port follows a patch at
/// a value: [`followed`] for the check, a narrower one for a test.
pub fn check(pe: &[u8], baseline: &[Reviewed], followed: impl Fn(u32) -> bool) -> Report {
    let mut report = Report::default();
    let found = operands(pe, |value| function_of(value).is_some());
    for (&(value, form), &count) in &found {
        let func = function_of(value).expect("operands keeps values in the table");
        let (place, port) = place(func, value);
        let reviewed = baseline.iter().any(|line| line.value == value && line.form == form && count <= line.count);
        let site = format!("{value:#x}, {}+{:#x}, {} x{count}", func.name, value as usize - func.ea, form.name());
        let failure = match place {
            Place::CensusOnly => None,
            Place::Entry if reviewed => None,
            Place::Entry => Some(format!("the entry of {}: a call, an entry jump or a vtable slot; not in the baseline", port.unwrap_or("?"))),
            Place::Head if head_target(func, value) && followed(value) => None,
            Place::Head => Some(format!("in the stolen bytes of {}: the detour will refuse the renderer", port.unwrap_or("?"))),
            Place::Body if followed(value) || ports::wraps_engine(func.ea) || reviewed => None,
            Place::Body => Some(format!("in the body of {}: no port follows a patch there, and the baseline does not have it", port.unwrap_or("?"))),
        };
        match failure {
            Some(why) => {
                let line = format!("FAIL: MWSE.dll names {site}, {why}");
                report.lines.push(line.clone());
                report.failures.push(line);
            }
            None => report.lines.push(format!("ok: {site} ({})", place.name())),
        }
    }
    for line in baseline {
        if !found.contains_key(&(line.value, line.form)) {
            report.stale.push(format!("stale: the baseline has {:#x} {} x{}, and the DLL does not", line.value, line.form.name(), line.count));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value of the site of MWSE e2b957c25d: the call of `dtor` in `deleting_dtor`.
    const DTOR_SITE: u32 = 0x6A9D83;

    /// A PE with a `.text` and a `.rdata` section of the given bytes.
    fn made_up_pe(text: &[u8], rdata: &[u8]) -> Vec<u8> {
        let header = 0x40;
        let table = header + 24;
        let raw = (table + 2 * 40 + 15) & !15;
        let mut pe = vec![0u8; raw];
        pe[0x3C..0x40].copy_from_slice(&(header as u32).to_le_bytes());
        pe[header..header + 4].copy_from_slice(b"PE\0\0");
        pe[header + 6..header + 8].copy_from_slice(&2u16.to_le_bytes());
        let mut at = raw;
        for (index, (name, bytes)) in [(&b".text\0\0\0"[..], text), (&b".rdata\0\0"[..], rdata)].into_iter().enumerate() {
            let entry = table + 40 * index;
            pe[entry..entry + 8].copy_from_slice(name);
            pe[entry + 16..entry + 20].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            pe[entry + 20..entry + 24].copy_from_slice(&(at as u32).to_le_bytes());
            at += bytes.len();
        }
        pe.extend_from_slice(text);
        pe.extend_from_slice(rdata);
        pe
    }

    fn le(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    #[test]
    fn a_push_of_a_site_is_an_operand_and_the_same_bytes_without_an_opcode_are_not() {
        let mut text = vec![0x90, 0x68];
        text.extend_from_slice(&le(DTOR_SITE));
        // The same 4 bytes after a byte that is no opcode of the three forms.
        text.extend_from_slice(&[0x90, 0x90]);
        text.extend_from_slice(&le(DTOR_SITE));
        // A `mov ecx, imm32` and a `mov dword [esp+8], imm32`.
        text.push(0xB9);
        text.extend_from_slice(&le(DTOR_SITE));
        text.extend_from_slice(&[0xC7, 0x44, 0x24, 0x08]);
        text.extend_from_slice(&le(DTOR_SITE));
        let found = operands(&made_up_pe(&text, &[]), |value| value == DTOR_SITE);
        assert_eq!(found.get(&(DTOR_SITE, Form::Push)), Some(&1));
        assert_eq!(found.get(&(DTOR_SITE, Form::MovReg)), Some(&1));
        assert_eq!(found.get(&(DTOR_SITE, Form::MovMem)), Some(&1));
        assert_eq!(found.values().sum::<usize>(), 3);
    }

    #[test]
    fn an_aligned_value_of_a_site_list_is_data() {
        let mut rdata = vec![0u8; 4];
        rdata.extend_from_slice(&le(0x6AD0ED));
        // Not aligned: no site list.
        rdata.extend_from_slice(&[0, 0]);
        rdata.extend_from_slice(&le(0x6AD0ED));
        rdata.extend_from_slice(&[0, 0]);
        let found = operands(&made_up_pe(&[], &rdata), |value| value == 0x6AD0ED);
        assert_eq!(found.get(&(0x6AD0ED, Form::Data)), Some(&1));
    }

    #[test]
    fn the_places_of_a_ported_function() {
        let func = function_of(0x6A9D80).unwrap();
        assert_eq!(func.name, "NiDX8Renderer::deleting_dtor");
        assert_eq!(place(func, 0x6A9D80).0, Place::Entry);
        assert_eq!(place(func, DTOR_SITE).0, Place::Head);
        assert_eq!(place(func, 0x6A9D90).0, Place::Body);
        assert!(head_target(func, DTOR_SITE));
        assert!(!head_target(func, 0x6A9D84));
        // NiAVObject::Update is in the table, and no port has it.
        assert_eq!(place(function_of(0x6EB000).unwrap(), 0x6EB000).0, Place::CensusOnly);
        assert!(function_of(0x401000).is_none());
    }

    #[test]
    fn the_redirected_dtor_call_passes_with_its_follow_and_fails_without_it() {
        let mut text = vec![0x68];
        text.extend_from_slice(&le(DTOR_SITE));
        let pe = made_up_pe(&text, &[]);
        let report = check(&pe, &[], followed);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        let report = check(&pe, &[], |value| value != DTOR_SITE && followed(value));
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].contains("0x6a9d83, NiDX8Renderer::deleting_dtor+0x3"), "{}", report.failures[0]);
        assert!(report.failures[0].contains("in the stolen bytes of NiDX8Renderer::deleting_dtor"), "{}", report.failures[0]);
    }

    #[test]
    fn an_entry_passes_only_by_its_form_and_count_in_the_baseline() {
        // `NiDX8Renderer::dtor` 0x6A9E10: MWSE names it as the expected callee of its call.
        let mut text = vec![0x68];
        text.extend_from_slice(&le(0x6A9E10));
        let pe = made_up_pe(&text, &[]);
        assert_eq!(check(&pe, &[], followed).failures.len(), 1);
        let push = Reviewed { value: 0x6A9E10, form: Form::Push, count: 1 };
        assert!(check(&pe, &[push.clone()], followed).failures.is_empty());
        // The same value in another form, or one more time, is a new occurrence.
        let mov = Reviewed { form: Form::MovReg, ..push.clone() };
        assert_eq!(check(&pe, &[mov], followed).failures.len(), 1);
        let mut twice = text.clone();
        twice.extend_from_slice(&text);
        assert_eq!(check(&made_up_pe(&twice, &[]), &[push], followed).failures.len(), 1);
    }

    #[test]
    fn a_baseline_line_needs_a_reason_and_a_missing_value_is_stale() {
        assert!(parse_baseline("0x6a9e10 push 2").is_err());
        let lines = parse_baseline("# a comment\n0x6a9e10 push 2 the expected callee of the call at 0x6A9D83\n").unwrap();
        assert_eq!(lines, vec![Reviewed { value: 0x6A9E10, form: Form::Push, count: 2 }]);
        let report = check(&made_up_pe(&[0x90; 8], &[]), &lines, followed);
        assert_eq!(report.stale.len(), 1);
        assert!(report.failures.is_empty());
    }

    /// The baseline of the repository.
    fn baseline() -> Vec<Reviewed> {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/mwse_sites.baseline")).expect("tools/mwse_sites.baseline");
        parse_baseline(&text).unwrap_or_else(|error| panic!("tools/mwse_sites.baseline: {error}"))
    }

    #[test]
    fn the_baseline_parses() {
        baseline();
    }

    /// The check of the DLL that `NIRENDER_MWSE_DLL` names (`tools/check_mwse_sites.py`).
    /// Skipped without the variable; with the variable and no file, it fails.
    #[test]
    fn every_mwse_site_in_a_port_is_followed() {
        let Ok(path) = std::env::var("NIRENDER_MWSE_DLL") else {
            eprintln!("skipped: no NIRENDER_MWSE_DLL");
            return;
        };
        let pe = std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let report = check(&pe, &baseline(), followed);
        for line in report.lines.iter().chain(&report.stale) {
            println!("{line}");
        }
        println!("{path}: {} places in ported or census functions, {} failed, {} stale baseline lines", report.lines.len(), report.failures.len(), report.stale.len());
        assert!(report.failures.is_empty(), "{} places of MWSE.dll are not followed:\n{}", report.failures.len(), report.failures.join("\n"));
    }
}
