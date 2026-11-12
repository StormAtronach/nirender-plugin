//! The check of an MWSE.dll's patch sites against the ports, before the DLL goes to a test
//! install (Brief 22, 2026-11-10; the decode of the code: the Architect's brief
//! mwse-site-decode-2026-11-10).
//!
//! MWSE gives the address of each patch site to its helpers as an operand of an instruction
//! (`push imm32`, `mov r32, imm32`, `mov [m], imm32`), and its site lists are aligned values in
//! its data. So the 32-bit operands of the built DLL that fall inside a function of the table
//! name the places that the DLL patches there. A rule for each place tells whether the port of
//! the function follows the patch; `tools/mwse_sites.baseline` holds the reviewed places that
//! are no patch of a port (calls of a port's entry, constants that are data).
//!
//! The code is decoded, not read as windows: a recursive descent ([`reached`]) from the entry
//! point, the exports and the relocated pointers into `.text`, with the crate iced-x86. A value
//! counts in a form only when it is the 32-bit immediate of a reached instruction of that form.
//! A value inside another instruction of a strong block (the target of a `call` and the next
//! byte) is no operand. A block is strong when a root reaches it over branches, calls and the
//! fall-throughs of calls that come back; else it is weak (the bytes after a call that never
//! comes back can be data). A value in the bytes of a weak block, or in bytes that the descent
//! does not reach (a table, padding, code that no known address reaches), counts in the form
//! `unreached` when the bytes before it can make it an operand (68, B8 to BF, C7): the safe side.
//!
//! The rules, in this order; the first that applies decides:
//! 1. The function has no port (census only): the place is listed, never failed.
//! 2. ENTRY, the place is the function's entry: it fails unless the baseline has it.
//! 3. HEAD, inside the bytes that the detour steals: it passes only at the `call` or `jmp` of a
//!    32-bit target that the head compare lets change (`hook::head_matches`), when a port serves
//!    the site or a known patch covers it.
//! 4. BODY, past the stolen bytes: it passes when a known patch, a served site or a port that
//!    runs the whole engine function covers it, or when it is an aligned value (`data`) that is
//!    the address of one of MWSE's count lock sites (`count_locks::is_site`: MWSE's table of
//!    those sites, which the census covers); else it fails unless the baseline has it.
//!
//! A baseline line is the value, the instruction form and its count: a new form or a higher
//! count of a value fails until it is reviewed.
//!
//! Limits: an address that MWSE computes at run time is no operand (the census of the game
//! start sees it); a patch that MWSE.json turns off still fails here (the safe side); a vtable
//! override is seen only when MWSE passes the function as the expected old value. Code that only
//! a computed address with no relocation reaches stays unreached, and its values count as
//! `unreached`.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};

use iced_x86::{Code, Decoder, DecoderOptions, FlowControl, Instruction, OpKind};

use crate::gen::funcs::{Func, FUNCS};
use crate::ports;

/// How the DLL holds a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Form {
    /// `push imm32`.
    Push,
    /// `mov r32, imm32`.
    MovReg,
    /// `mov r/m32, imm32` (C7 /0).
    MovMem,
    /// An aligned value in a section that is not code: a site list.
    Data,
    /// 4 bytes of `.text` that no instruction of a strong block holds, where the bytes before them
    /// can make them the immediate of one of the three forms.
    Unreached,
}

impl Form {
    pub fn name(self) -> &'static str {
        match self {
            Form::Push => "push",
            Form::MovReg => "mov-r32",
            Form::MovMem => "mov-m",
            Form::Data => "data",
            Form::Unreached => "unreached",
        }
    }

    pub fn parse(text: &str) -> Option<Form> {
        [Form::Push, Form::MovReg, Form::MovMem, Form::Data, Form::Unreached]
            .into_iter()
            .find(|form| form.name() == text)
    }

    /// The form of a decoded instruction whose immediate is 32 bits.
    fn of(code: Code) -> Option<Form> {
        match code {
            Code::Pushd_imm32 => Some(Form::Push),
            Code::Mov_r32_imm32 => Some(Form::MovReg),
            Code::Mov_rm32_imm32 => Some(Form::MovMem),
            _ => None,
        }
    }
}

/// A section of a PE file: the name, the RVA and the raw bytes.
pub struct Section<'a> {
    pub name: String,
    pub rva: u32,
    pub bytes: &'a [u8],
}

/// What the check needs of a PE32 file.
pub struct Pe<'a> {
    pub image_base: u32,
    /// The RVA of the entry point; 0 for none.
    pub entry: u32,
    pub sections: Vec<Section<'a>>,
    /// The RVAs of the exported functions.
    pub exports: Vec<u32>,
    /// The RVAs of the 32-bit base relocations (HIGHLOW), in order.
    pub relocs: Vec<u32>,
}

impl<'a> Pe<'a> {
    /// The sections, and from a PE32 optional header the image base, the entry point, the exports and the relocations.
    pub fn parse(pe: &'a [u8]) -> Option<Pe<'a>> {
        let word = |at: usize| Some(u16::from_le_bytes(pe.get(at..at + 2)?.try_into().ok()?) as usize);
        let dword = |at: usize| Some(u32::from_le_bytes(pe.get(at..at + 4)?.try_into().ok()?));
        let header = dword(0x3C)? as usize;
        let count = word(header + 6)?;
        let optional_size = word(header + 20)?;
        let optional = header + 24;
        let table = optional + optional_size;
        let mut sections = Vec::new();
        for index in 0..count {
            let entry = table + 40 * index;
            let name = String::from_utf8_lossy(pe.get(entry..entry + 8)?)
                .trim_end_matches('\0')
                .to_string();
            let rva = dword(entry + 12)?;
            let (size, raw) = (dword(entry + 16)? as usize, dword(entry + 20)? as usize);
            sections.push(Section {
                name,
                rva,
                bytes: &pe[raw.min(pe.len())..(raw + size).min(pe.len())],
            });
        }
        let mut image = Pe {
            image_base: 0,
            entry: 0,
            sections,
            exports: Vec::new(),
            relocs: Vec::new(),
        };
        // A PE32 optional header (magic 0x10B) with its data directories.
        if optional_size >= 96 && word(optional)? == 0x10B {
            image.entry = dword(optional + 16)?;
            image.image_base = dword(optional + 28)?;
            let directories = dword(optional + 92)? as usize;
            let directory = |index: usize| {
                let at = optional + 96 + 8 * index;
                (index < directories && at + 8 <= table)
                    .then(|| Some((dword(at)?, dword(at + 4)?)))
                    .flatten()
                    .filter(|&(rva, _)| rva != 0)
            };
            if let Some((rva, size)) = directory(0) {
                image.exports = image.export_rvas(rva, size);
            }
            if let Some((rva, size)) = directory(5) {
                image.relocs = image.reloc_rvas(rva, size);
            }
        }
        Some(image)
    }

    fn bytes_at(&self, rva: u32, len: usize) -> Option<&'a [u8]> {
        self.sections.iter().find_map(|section| {
            let offset = rva.checked_sub(section.rva)? as usize;
            section.bytes.get(offset..offset + len)
        })
    }

    fn dword_at(&self, rva: u32) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes_at(rva, 4)?.try_into().ok()?))
    }

    /// The functions of the export table; a forwarder (an RVA inside the table) is no function.
    fn export_rvas(&self, rva: u32, size: u32) -> Vec<u32> {
        let (Some(count), Some(functions)) = (self.dword_at(rva + 20), self.dword_at(rva + 28)) else {
            return Vec::new();
        };
        (0..count)
            .filter_map(|index| self.dword_at(functions + 4 * index))
            .filter(|&function| function != 0 && !(rva..rva + size).contains(&function))
            .collect()
    }

    /// The HIGHLOW entries of the base relocation table.
    fn reloc_rvas(&self, rva: u32, size: u32) -> Vec<u32> {
        let mut found = Vec::new();
        let mut at = rva;
        while at + 8 <= rva + size {
            let (Some(page), Some(block)) = (self.dword_at(at), self.dword_at(at + 4)) else {
                break;
            };
            let Some(entries) = block.checked_sub(8).and_then(|length| self.bytes_at(at + 8, length as usize)) else {
                break;
            };
            for pair in entries.chunks_exact(2) {
                let entry = u16::from_le_bytes([pair[0], pair[1]]);
                if entry >> 12 == 3 {
                    found.push(page + u32::from(entry & 0xFFF));
                }
            }
            at += block.max(8);
        }
        found.sort_unstable();
        found
    }
}

/// The form of the 4 bytes at `at` of the code `text` when the bytes before them can make them the
/// immediate of one of the three instructions; else None. The filter of the bytes that no strong
/// instruction holds: a value that no 68, B8 to BF or C7 can own is no operand there.
fn code_form(text: &[u8], at: usize) -> Option<Form> {
    let before = |back: usize| if at >= back { Some(text[at - back]) } else { None };
    match before(1) {
        Some(0x68) => return Some(Form::Push),
        Some(0xB8..=0xBF) => return Some(Form::MovReg),
        _ => {}
    }
    // `mov r/m32, imm32` (C7 /0): C7, a ModR/M byte, a SIB byte when the ModR/M asks for one, a
    // displacement of 0, 1 or 4 bytes, then the value. So the C7 is 2 to 7 bytes before it.
    if (2..=7).any(|back| at >= back && text[at - back] == 0xC7 && mov_imm_offset(&text[at - back..]) == Some(back)) {
        return Some(Form::MovMem);
    }
    None
}

/// The offset of the 32-bit value in a `mov r/m32, imm32` at the start of `code` (C7, then the
/// ModR/M byte with the register field 0), from the ModR/M and SIB bytes; None for another
/// instruction.
fn mov_imm_offset(code: &[u8]) -> Option<usize> {
    let modrm = *code.get(1)?;
    let (mode, reg, rm) = (modrm >> 6, (modrm >> 3) & 7, modrm & 7);
    if code[0] != 0xC7 || reg != 0 {
        return None;
    }
    if mode == 3 {
        return Some(2);
    }
    let sib = rm == 4;
    let base = if sib { *code.get(2)? & 7 } else { rm };
    let displacement = match mode {
        0 if base == 5 => 4,
        0 => 0,
        1 => 1,
        _ => 4,
    };
    Some(2 + sib as usize + displacement)
}

/// How sure the decode is that a block is code. A strong block is reached from a root (a seed or
/// a table entry) over branches, calls, code addresses and the fall-throughs of calls that come
/// back. A weak block is reached only over the fall-through of a call that may never come back:
/// the bytes after such a call can be data, and a branch in them can be garbage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Strength {
    Weak,
    Strong,
}

/// A way from a block to a block start.
#[derive(Debug, Clone, Copy)]
enum Edge {
    /// A branch target, or the next instruction where another block starts: the flow goes on there.
    Flow(u32),
    /// A call target, or a relocated code address in an immediate (`push offset func`).
    Address(u32),
    /// The fall-through after a call; `callee` is the call target in the code, None for an
    /// indirect call or a target outside the code.
    After { start: u32, callee: Option<u32> },
}

/// A decoded instruction: its offset in `.text`, its length, the offsets in it of its 32-bit
/// constants (the immediate, the displacement; 0 for none), and its form when its immediate is
/// one of the three.
#[derive(Debug, Clone, Copy)]
struct Insn {
    start: u32,
    len: u8,
    constants: [u8; 2],
    form: Option<Form>,
}

/// The instructions of a block from one start and its ways out; `data` when the relocation test
/// dropped it; `leaves` when it ends in a return or an indirect jump (the flow can go back to a
/// caller).
struct Block {
    start: u32,
    insns: Vec<Insn>,
    edges: Vec<Edge>,
    data: bool,
    leaves: bool,
}

/// The near target of a branch, when it has one.
fn near_target(instruction: &Instruction) -> Option<u64> {
    matches!(instruction.op0_kind(), OpKind::NearBranch16 | OpKind::NearBranch32).then(|| instruction.near_branch_target())
}

/// Decodes a block from `start` of the code `text` (at the address `va`): up to a return, a
/// jump, an interrupt, a call, an invalid byte or the start of another block. `reloc` has, for
/// each byte of a relocated dword, 1 + the offset of the dword. A block in which a relocated
/// dword lies in an instruction and is none of its constants is data: it has no edges.
fn decode_block(text: &[u8], va: u32, start: u32, starts: &HashSet<u32>, reloc: &[u32]) -> Block {
    let offset_of = |address: u64| {
        address
            .checked_sub(u64::from(va))
            .filter(|&offset| offset < text.len() as u64)
            .map(|offset| offset as u32)
    };
    let mut decoder = Decoder::with_ip(32, text, u64::from(va), DecoderOptions::NONE);
    decoder.set_position(start as usize).expect("a block start is in the code");
    decoder.set_ip(u64::from(va) + u64::from(start));
    let mut block = Block {
        start,
        insns: Vec::new(),
        edges: Vec::new(),
        data: false,
        leaves: false,
    };
    let mut instruction = Instruction::default();
    while decoder.can_decode() {
        let at = decoder.position() as u32;
        if at != start && starts.contains(&at) {
            block.edges.push(Edge::Flow(at));
            break;
        }
        decoder.decode_out(&mut instruction);
        if instruction.is_invalid() {
            break;
        }
        let len = instruction.len() as u32;
        let offsets = decoder.get_constant_offsets(&instruction);
        let mut constants = [0u8; 2];
        if offsets.has_immediate() && offsets.immediate_size() == 4 {
            constants[0] = offsets.immediate_offset() as u8;
        }
        if offsets.has_displacement() && offsets.displacement_size() == 4 {
            constants[1] = offsets.displacement_offset() as u8;
        }
        let constant_at = |dword: u32| constants.iter().any(|&offset| offset != 0 && at + u32::from(offset) == dword);
        if (at..at + len).any(|byte| reloc[byte as usize] != 0 && !constant_at(reloc[byte as usize] - 1)) {
            block.data = true;
            block.edges.clear();
            return block;
        }
        // A relocated immediate that points into the code is a code address (`push offset func`).
        let immediate = at + u32::from(constants[0]);
        if constants[0] != 0 && reloc[immediate as usize] == immediate + 1 {
            if let Some(target) = offset_of(u64::from(instruction.immediate32())) {
                block.edges.push(Edge::Address(target));
            }
        }
        let form = Form::of(instruction.code()).filter(|_| constants[0] != 0);
        block.insns.push(Insn {
            start: at,
            len: len as u8,
            constants,
            form,
        });
        let target = near_target(&instruction).and_then(offset_of);
        match instruction.flow_control() {
            FlowControl::Next | FlowControl::XbeginXabortXend => {}
            FlowControl::ConditionalBranch => block.edges.extend(target.map(Edge::Flow)),
            FlowControl::UnconditionalBranch => {
                block.edges.extend(target.map(Edge::Flow));
                break;
            }
            FlowControl::Call | FlowControl::IndirectCall => {
                block.edges.extend(target.map(Edge::Address));
                if ((at + len) as usize) < text.len() {
                    block.edges.push(Edge::After {
                        start: at + len,
                        callee: target,
                    });
                }
                break;
            }
            FlowControl::Return | FlowControl::IndirectBranch => {
                block.leaves = true;
                break;
            }
            FlowControl::Interrupt | FlowControl::Exception => break,
        }
    }
    block
}

/// What the descent reached in `.text`.
pub struct Decode {
    /// For each byte: 1 + the offset of the start of its instruction, or 0 when no kept
    /// instruction has it.
    owner: Vec<u32>,
    /// For each byte: true when a kept instruction of a strong block has it.
    strong: Vec<bool>,
    /// For each byte: true when a 32-bit constant of a kept instruction starts there.
    constant: Vec<bool>,
    /// For each byte: the form of the kept instruction whose 32-bit immediate starts there.
    immediate: Vec<Option<Form>>,
    /// The blocks decoded, and those that the relocation test dropped.
    pub blocks: usize,
    pub dropped: usize,
    /// The offsets in `.text` where two blocks of the same strength decode the same bytes at
    /// other instruction starts: both are out.
    pub overlaps: Vec<u32>,
}

impl Decode {
    /// The bytes of `.text` that a kept instruction holds.
    pub fn reached(&self) -> usize {
        self.owner.iter().filter(|&&owner| owner != 0).count()
    }

    /// The bytes of `.text` that only a kept instruction of a weak block holds.
    pub fn weak(&self) -> usize {
        self.owner
            .iter()
            .zip(&self.strong)
            .filter(|&(&owner, &strong)| owner != 0 && !strong)
            .count()
    }
}

/// The blocks of a descent, with the block of each start and the roots (the seeds and the table
/// entries).
struct Graph {
    blocks: Vec<Block>,
    index: HashMap<u32, usize>,
    roots: HashSet<u32>,
}

impl Graph {
    /// For each block, the way back to a caller: true when a return or an indirect jump is
    /// reachable from it over flow edges and over the fall-throughs of calls that come back
    /// themselves (a call with no known callee comes back). Only the blocks that are `live`.
    fn comes_back(&self, live: &[bool]) -> Vec<bool> {
        let count = self.blocks.len();
        // The blocks that lead into each block: by flow (None), or by the fall-through of a call of a known callee.
        let mut before: Vec<Vec<(usize, Option<u32>)>> = vec![Vec::new(); count];
        for (index, block) in self.blocks.iter().enumerate().filter(|&(index, _)| live[index]) {
            for edge in &block.edges {
                let (target, callee) = match *edge {
                    Edge::Flow(target) => (target, None),
                    Edge::After { start, callee } => (start, callee),
                    Edge::Address(_) => continue,
                };
                if let Some(&next) = self.index.get(&target) {
                    before[next].push((index, callee));
                }
            }
        }
        let mut back = vec![false; count];
        let mut work: Vec<usize> = (0..count).filter(|&index| live[index] && self.blocks[index].leaves).collect();
        for &index in &work {
            back[index] = true;
        }
        // The callers whose fall-through comes back, waiting for their callee.
        let mut waiting: HashMap<u32, Vec<usize>> = HashMap::new();
        while let Some(index) = work.pop() {
            // The callers that waited for this block as their callee come back now.
            let mut callers = waiting.remove(&self.blocks[index].start).unwrap_or_default();
            for &(caller, callee) in &before[index] {
                match callee {
                    Some(callee) if !self.index.get(&callee).is_some_and(|&called| back[called]) => {
                        waiting.entry(callee).or_default().push(caller)
                    }
                    _ => callers.push(caller),
                }
            }
            for caller in callers {
                if live[caller] && !back[caller] {
                    back[caller] = true;
                    work.push(caller);
                }
            }
        }
        back
    }

    /// The strength of each block that is `live` and that a root reaches over live blocks; None
    /// for the others (a block whose every parent is out is out too).
    fn strengths(&self, live: &[bool]) -> Vec<Option<Strength>> {
        let back = self.comes_back(live);
        let live_at = |start: u32| self.index.get(&start).copied().filter(|&index| live[index]);
        let mut strength = vec![None; self.blocks.len()];
        let mut work: Vec<usize> = self.roots.iter().filter_map(|&root| live_at(root)).collect();
        for &index in &work {
            strength[index] = Some(Strength::Strong);
        }
        // The strong blocks, then the weak blocks that the strong ones reach.
        for pass in [Strength::Strong, Strength::Weak] {
            if pass == Strength::Weak {
                work = (0..self.blocks.len()).filter(|&index| strength[index].is_some()).collect();
            }
            while let Some(index) = work.pop() {
                for edge in &self.blocks[index].edges {
                    let (target, strong) = match *edge {
                        Edge::Flow(target) | Edge::Address(target) => (target, true),
                        Edge::After { start, callee } => (
                            start,
                            callee.is_none_or(|callee| live_at(callee).is_some_and(|called| back[called])),
                        ),
                    };
                    if let Some(next) =
                        live_at(target).filter(|&next| strength[next].is_none() && (strong || pass == Strength::Weak))
                    {
                        strength[next] = Some(pass);
                        work.push(next);
                    }
                }
            }
        }
        strength
    }

    /// Keeps the blocks that are not data, that a root reaches and that do not clash: where two
    /// kept blocks decode the same bytes at other instruction starts, a strong block wins over a
    /// weak one, and two of the same strength both go out. A block out takes out what only it
    /// reaches, and the strengths are computed again, until no clash is left.
    fn resolve(&self, length: usize) -> Decode {
        let blocks = &self.blocks;
        let mut live: Vec<bool> = blocks.iter().map(|block| !block.data).collect();
        let mut overlaps = Vec::new();
        let (owner, strong, strength) = loop {
            let strength = self.strengths(&live);
            let mut order: Vec<usize> = (0..blocks.len()).filter(|&index| strength[index].is_some()).collect();
            order.sort_by_key(|&index| Reverse(strength[index]));
            let mut owner = vec![0u32; length];
            let mut strong = vec![false; length];
            let mut owner_block = vec![u32::MAX; length];
            let mut clashes = Vec::new();
            for &index in &order {
                let insns = &blocks[index].insns;
                let clash = insns.iter().find_map(|insn| {
                    (insn.start..insn.start + u32::from(insn.len))
                        .find(|&byte| owner[byte as usize] != 0 && owner[byte as usize] != insn.start + 1)
                        .map(|byte| (owner_block[byte as usize] as usize, byte))
                });
                if let Some((other, byte)) = clash {
                    clashes.push((other, index, byte));
                    continue;
                }
                for insn in insns {
                    for byte in insn.start..insn.start + u32::from(insn.len) {
                        if owner[byte as usize] == 0 {
                            owner[byte as usize] = insn.start + 1;
                            strong[byte as usize] = strength[index] == Some(Strength::Strong);
                            owner_block[byte as usize] = index as u32;
                        }
                    }
                }
            }
            if clashes.is_empty() {
                break (owner, strong, strength);
            }
            // The blocks go in order of strength, so the one that holds the bytes is as strong or stronger.
            for (other, index, byte) in clashes {
                live[index] = false;
                if strength[other] == strength[index] {
                    live[other] = false;
                    overlaps.push(byte);
                }
            }
        };
        let mut constant = vec![false; length];
        let mut immediate = vec![None; length];
        for insn in (0..blocks.len())
            .filter(|&index| strength[index].is_some())
            .flat_map(|index| &blocks[index].insns)
        {
            for offset in insn.constants.into_iter().filter(|&offset| offset != 0) {
                constant[(insn.start + u32::from(offset)) as usize] = true;
            }
            if let Some(form) = insn.form {
                immediate[(insn.start + u32::from(insn.constants[0])) as usize] = Some(form);
            }
        }
        overlaps.sort_unstable();
        overlaps.dedup();
        Decode {
            owner,
            strong,
            constant,
            immediate,
            blocks: blocks.len(),
            dropped: blocks.iter().filter(|block| block.data).count(),
            overlaps,
        }
    }
}

/// The recursive descent over `.text` (the brief's steps 1 to 8): from the entry point, the
/// exports and the relocated pointers into the code from other sections; then the branch
/// targets, the fall-throughs and the relocated code addresses; then, to a fixed point, the
/// entries of the jump tables: each relocated dword of `.text` that no kept instruction has as
/// a constant, and that points into the code. The seeds and the table entries are the roots.
pub fn reached(pe: &Pe) -> Option<Decode> {
    let text = pe.sections.iter().find(|section| section.name == ".text")?;
    let bytes = text.bytes;
    let va = pe.image_base.wrapping_add(text.rva);
    let in_text = |rva: u32| rva.checked_sub(text.rva).filter(|&offset| (offset as usize) < bytes.len());
    let code_offset = |value: u32| in_text(value.wrapping_sub(pe.image_base));
    // Each byte of a relocated dword holds 1 + the offset of the dword.
    let mut reloc = vec![0u32; bytes.len()];
    for offset in pe.relocs.iter().filter_map(|&rva| in_text(rva)) {
        for byte in offset..(offset + 4).min(bytes.len() as u32) {
            reloc[byte as usize] = offset + 1;
        }
    }
    let mut graph = Graph {
        blocks: Vec::new(),
        index: HashMap::new(),
        roots: HashSet::new(),
    };
    let mut starts = HashSet::new();
    let mut work = Vec::new();
    let add_root = |graph: &mut Graph, starts: &mut HashSet<u32>, work: &mut Vec<u32>, offset: u32| {
        graph.roots.insert(offset);
        if starts.insert(offset) {
            work.push(offset);
        }
    };
    for offset in [pe.entry].into_iter().chain(pe.exports.iter().copied()).filter_map(in_text) {
        add_root(&mut graph, &mut starts, &mut work, offset);
    }
    for &rva in pe.relocs.iter().filter(|&&rva| in_text(rva).is_none()) {
        if let Some(offset) = pe.dword_at(rva).and_then(code_offset) {
            add_root(&mut graph, &mut starts, &mut work, offset);
        }
    }
    loop {
        while let Some(start) = work.pop() {
            let block = decode_block(bytes, va, start, &starts, &reloc);
            for edge in &block.edges {
                let target = match *edge {
                    Edge::Flow(target) | Edge::Address(target) | Edge::After { start: target, .. } => target,
                };
                if starts.insert(target) {
                    work.push(target);
                }
            }
            graph.index.insert(start, graph.blocks.len());
            graph.blocks.push(block);
        }
        let decode = graph.resolve(bytes.len());
        let roots = graph.roots.len();
        for offset in (0..bytes.len() as u32)
            .filter(|&offset| reloc[offset as usize] == offset + 1 && !decode.constant[offset as usize])
        {
            let value = bytes
                .get(offset as usize..offset as usize + 4)
                .map(|word| u32::from_le_bytes(word.try_into().unwrap()));
            if let Some(target) = value.and_then(code_offset) {
                add_root(&mut graph, &mut starts, &mut work, target);
            }
        }
        if graph.roots.len() == roots {
            return Some(decode);
        }
    }
}

/// The values of `pe` for which `inside` is true, by form, with the number of their occurrences,
/// and the reach of the decode.
pub fn operands(pe: &[u8], inside: impl Fn(u32) -> bool) -> (BTreeMap<(u32, Form), usize>, Reach) {
    let mut found = BTreeMap::new();
    let mut reach = Reach::default();
    let Some(image) = Pe::parse(pe) else {
        return (found, reach);
    };
    let decode = reached(&image);
    for section in &image.sections {
        let bytes = section.bytes;
        let code = section.name == ".text";
        let step = if code { 1 } else { 4 };
        let mut at = 0;
        while at + 4 <= bytes.len() {
            let value = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            if inside(value) {
                let form = match &decode {
                    _ if !code => Some(Form::Data),
                    Some(decode) if decode.immediate[at].is_some() => decode.immediate[at],
                    Some(decode) if decode.strong[at] => {
                        reach.removed += 1;
                        None
                    }
                    // Bytes of a weak block, or that no block reaches: a value counts where the bytes before it can make it an operand.
                    _ => code_form(bytes, at).map(|_| Form::Unreached),
                };
                if let Some(form) = form {
                    *found.entry((value, form)).or_insert(0) += 1;
                }
            }
            at += step;
        }
        if code {
            reach.text = bytes.len();
        }
    }
    if let Some(decode) = &decode {
        reach.reached = decode.reached();
        reach.weak = decode.weak();
        reach.blocks = decode.blocks;
        reach.dropped = decode.dropped;
        let va = image
            .sections
            .iter()
            .find(|section| section.name == ".text")
            .map_or(0, |text| image.image_base.wrapping_add(text.rva));
        reach.overlaps = decode.overlaps.iter().map(|&offset| va.wrapping_add(offset)).collect();
    }
    (found, reach)
}

/// The reach of the decode, for the report: a fall from one build to the next shows at once.
#[derive(Debug, Default)]
pub struct Reach {
    /// The bytes of `.text`, those that a kept instruction holds, and those of them that only a
    /// weak block holds.
    pub text: usize,
    pub reached: usize,
    pub weak: usize,
    /// The blocks decoded, and those that the relocation test dropped.
    pub blocks: usize,
    pub dropped: usize,
    /// The addresses where two blocks of one strength disagree.
    pub overlaps: Vec<u32>,
    /// The values in a table function that lie inside an instruction of a strong block and are
    /// not its immediate: no instruction holds them.
    pub removed: usize,
}

impl Reach {
    pub fn line(&self) -> String {
        let share = if self.text == 0 {
            0.0
        } else {
            100.0 * self.reached as f64 / self.text as f64
        };
        let overlaps: Vec<String> = self.overlaps.iter().take(8).map(|address| format!("{address:#x}")).collect();
        format!(
            "reach: {} of {} bytes of .text ({share:.1} %, {} of them weak), {} blocks, {} dropped as data, {} overlaps{}{}, {} values inside other instructions removed",
            self.reached,
            self.text,
            self.weak,
            self.blocks,
            self.dropped,
            self.overlaps.len(),
            if overlaps.is_empty() { String::new() } else { format!(" ({}", overlaps.join(", ")) },
            if overlaps.is_empty() { "" } else if self.overlaps.len() > 8 { ", ...)" } else { ")" },
            self.removed
        )
    }
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

/// True when `value`, held as `form`, is an entry of MWSE's table of count lock sites: an aligned
/// value that is the address of a site. The census does not count MWSE's lock at that site as a
/// change of another mod ([`crate::count_locks::covers`]). The same address in another form is a
/// patch of another kind and is not covered.
fn count_lock_site(value: u32, form: Form) -> bool {
    form == Form::Data && crate::count_locks::is_site(value as usize)
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
    /// The reach of the decode.
    pub reach: Reach,
}

/// Checks the operands of `pe` by the rules. `followed` tells whether a port follows a patch at
/// a value: [`followed`] for the check, a narrower one for a test.
pub fn check(pe: &[u8], baseline: &[Reviewed], followed: impl Fn(u32) -> bool) -> Report {
    let mut report = Report::default();
    let (found, reach) = operands(pe, |value| function_of(value).is_some());
    report.reach = reach;
    for (&(value, form), &count) in &found {
        let func = function_of(value).expect("operands keeps values in the table");
        let (place, port) = place(func, value);
        let reviewed = baseline
            .iter()
            .any(|line| line.value == value && line.form == form && count <= line.count);
        let lock_site = count_lock_site(value, form);
        let site = format!(
            "{value:#x}, {}+{:#x}, {} x{count}",
            func.name,
            value as usize - func.ea,
            form.name()
        );
        let failure = match place {
            Place::CensusOnly => None,
            Place::Entry if reviewed => None,
            Place::Entry => Some(format!(
                "the entry of {}: a call, an entry jump or a vtable slot; not in the baseline",
                port.unwrap_or("?")
            )),
            Place::Head if head_target(func, value) && followed(value) => None,
            Place::Head => Some(format!(
                "in the stolen bytes of {}: the detour will refuse the renderer",
                port.unwrap_or("?")
            )),
            Place::Body if followed(value) || ports::wraps_engine(func.ea) || lock_site || reviewed => None,
            Place::Body => Some(format!(
                "in the body of {}: no port follows a patch there, and the baseline does not have it",
                port.unwrap_or("?")
            )),
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
            report.stale.push(format!(
                "stale: the baseline has {:#x} {} x{}, and the DLL does not",
                line.value,
                line.form.name(),
                line.count
            ));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value of the site of MWSE e2b957c25d: the call of `dtor` in `deleting_dtor`.
    const DTOR_SITE: u32 = 0x6A9D83;
    const IMAGE_BASE: u32 = 0x1000_0000;
    const TEXT_RVA: u32 = 0x1000;

    /// The address of the byte `offset` of the `.text` of [`made_up_pe`].
    fn va(offset: u32) -> u32 {
        IMAGE_BASE + TEXT_RVA + offset
    }

    /// A PE32 image at 0x10000000: `.text` at RVA 0x1000 with the entry point at its start,
    /// `.rdata` after it, and a `.reloc` section with a HIGHLOW entry for each of `relocs`
    /// (offsets in `.text`).
    fn made_up_pe(text: &[u8], rdata: &[u8], relocs: &[u32]) -> Vec<u8> {
        let page = |length: usize| (length.max(1) as u32).div_ceil(0x1000) * 0x1000;
        let rdata_rva = TEXT_RVA + page(text.len());
        let reloc_rva = rdata_rva + page(rdata.len());
        let mut reloc_table = Vec::new();
        for &offset in relocs {
            let rva = TEXT_RVA + offset;
            reloc_table.extend_from_slice(&(rva & !0xFFF).to_le_bytes());
            reloc_table.extend_from_slice(&12u32.to_le_bytes());
            reloc_table.extend_from_slice(&(0x3000u16 | (rva & 0xFFF) as u16).to_le_bytes());
            reloc_table.extend_from_slice(&0u16.to_le_bytes());
        }
        let header = 0x40;
        let optional = header + 24;
        let optional_size = 224;
        let table = optional + optional_size;
        let raw = table + 3 * 40;
        let mut pe = vec![0u8; raw];
        let mut put = |at: usize, bytes: &[u8]| pe[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x3C, &(header as u32).to_le_bytes());
        put(header, b"PE\0\0");
        put(header + 4, &0x14Cu16.to_le_bytes());
        put(header + 6, &3u16.to_le_bytes());
        put(header + 20, &(optional_size as u16).to_le_bytes());
        put(optional, &0x10Bu16.to_le_bytes());
        put(optional + 16, &TEXT_RVA.to_le_bytes());
        put(optional + 28, &IMAGE_BASE.to_le_bytes());
        put(optional + 92, &16u32.to_le_bytes());
        put(optional + 96 + 40, &reloc_rva.to_le_bytes());
        put(optional + 96 + 44, &(reloc_table.len() as u32).to_le_bytes());
        let mut at = raw;
        for (index, (name, rva, bytes)) in [
            (&b".text\0\0\0"[..], TEXT_RVA, text),
            (b".rdata\0\0", rdata_rva, rdata),
            (b".reloc\0\0", reloc_rva, &reloc_table[..]),
        ]
        .into_iter()
        .enumerate()
        {
            let entry = table + 40 * index;
            put(entry, name);
            put(entry + 8, &(bytes.len() as u32).to_le_bytes());
            put(entry + 12, &rva.to_le_bytes());
            put(entry + 16, &(bytes.len() as u32).to_le_bytes());
            put(entry + 20, &(at as u32).to_le_bytes());
            at += bytes.len();
        }
        pe.extend_from_slice(text);
        pe.extend_from_slice(rdata);
        pe.extend_from_slice(&reloc_table);
        pe
    }

    fn le(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    fn code(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    fn only(value: u32) -> impl Fn(u32) -> bool {
        move |found| found == value
    }

    #[test]
    fn the_three_forms_are_counted_and_a_displacement_is_not() {
        let text = code(&[
            &[0x68],
            &le(DTOR_SITE), // push imm32
            &[0xB9],
            &le(DTOR_SITE), // mov ecx, imm32
            &[0xC7, 0x44, 0x24, 0x08],
            &le(DTOR_SITE), // mov dword [esp+8], imm32
            &[0xA1],
            &le(DTOR_SITE), // mov eax, [DTOR_SITE]: a displacement
            &[0xC3],
        ]);
        let (found, reach) = operands(&made_up_pe(&text, &[], &[]), only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Push)), Some(&1));
        assert_eq!(found.get(&(DTOR_SITE, Form::MovReg)), Some(&1));
        assert_eq!(found.get(&(DTOR_SITE, Form::MovMem)), Some(&1));
        assert_eq!(found.values().sum::<usize>(), 3);
        assert_eq!(reach.removed, 1);
        assert_eq!(reach.reached, text.len());
    }

    #[test]
    fn a_mov_to_memory_is_found_in_each_of_its_forms() {
        let forms: [&[u8]; 7] = [
            &[0xC7, 0xC1],                               // mov ecx, imm32 (register form)
            &[0xC7, 0x00],                               // mov [eax], imm32
            &[0xC7, 0x45, 0xF8],                         // mov [ebp-8], imm32
            &[0xC7, 0x44, 0x24, 0x08],                   // mov [esp+8], imm32
            &[0xC7, 0x05, 0x10, 0x20, 0x7C, 0x00],       // mov [0x7C2010], imm32 (disp32)
            &[0xC7, 0x84, 0x24, 0x00, 0x01, 0x00, 0x00], // mov [esp+0x100], imm32 (SIB, disp32)
            &[0xC7, 0x04, 0x85, 0x00, 0x10, 0x7C, 0x00], // mov [eax*4+0x7C1000], imm32 (SIB, no base)
        ];
        for head in forms {
            let text = code(&[head, &le(DTOR_SITE), &[0xC3]]);
            let (found, _) = operands(&made_up_pe(&text, &[], &[]), only(DTOR_SITE));
            assert_eq!(found.get(&(DTOR_SITE, Form::MovMem)), Some(&1), "{head:02x?}");
        }
        // The value as the displacement of a mov to memory is no operand.
        let text = code(&[&[0xC7, 0x05], &le(DTOR_SITE), &le(1), &[0xC3]]);
        let (found, reach) = operands(&made_up_pe(&text, &[], &[]), only(DTOR_SITE));
        assert!(found.is_empty(), "{found:?}");
        assert_eq!(reach.removed, 1);
    }

    #[test]
    fn an_aligned_value_of_a_site_list_is_data() {
        let mut rdata = vec![0u8; 4];
        rdata.extend_from_slice(&le(0x6AD0ED));
        // Not aligned: no site list.
        rdata.extend_from_slice(&[0, 0]);
        rdata.extend_from_slice(&le(0x6AD0ED));
        rdata.extend_from_slice(&[0, 0]);
        let (found, _) = operands(&made_up_pe(&[0xC3], &rdata, &[]), only(0x6AD0ED));
        assert_eq!(found.get(&(0x6AD0ED, Form::Data)), Some(&1));
    }

    #[test]
    fn a_window_across_a_call_target_and_the_next_instruction_is_no_operand() {
        // call rel32 (00 68 F0 FE), push 0, ret: the bytes F0 FE 6A 00 are 0x6AFEF0, after a 0x68.
        let text = [0xE8, 0x00, 0x68, 0xF0, 0xFE, 0x6A, 0x00, 0xC3];
        let (found, reach) = operands(&made_up_pe(&text, &[], &[]), only(0x6AFEF0));
        assert!(found.is_empty(), "{found:?}");
        assert_eq!(reach.removed, 1);
    }

    #[test]
    fn a_push_after_a_modrm_byte_e8_is_counted() {
        // mov eax, [ebp-0x18], then push imm32: the E8 is a displacement byte, not a call.
        let text = code(&[&[0x8B, 0x45, 0xE8, 0x68], &le(DTOR_SITE), &[0xC3]]);
        let (found, _) = operands(&made_up_pe(&text, &[], &[]), only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Push)), Some(&1));
    }

    #[test]
    fn a_value_that_no_start_reaches_is_unreached_and_fails() {
        // ret, then a push that nothing reaches.
        let text = code(&[&[0xC3, 0x68], &le(DTOR_SITE), &[0xC3]]);
        let pe = made_up_pe(&text, &[], &[]);
        let (found, _) = operands(&pe, only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Unreached)), Some(&1));
        let report = check(&pe, &[], |value| value != DTOR_SITE && followed(value));
        assert_eq!(report.failures.len(), 1, "{:?}", report.lines);
        assert!(report.failures[0].contains("unreached"), "{}", report.failures[0]);
    }

    #[test]
    fn a_push_in_a_case_of_a_jump_table_is_counted() {
        // jmp [eax*4 + table]; the table of two cases; case 1: ret; case 2: push site, ret.
        let table = 7;
        let (case1, case2) = (15, 16);
        let text = code(&[
            &[0xFF, 0x24, 0x85],
            &le(va(table)),
            &le(va(case1)),
            &le(va(case2)),
            &[0xC3, 0x68],
            &le(DTOR_SITE),
            &[0xC3],
        ]);
        let (found, reach) = operands(&made_up_pe(&text, &[], &[3, table, table + 4]), only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Push)), Some(&1), "{reach:?}");
    }

    #[test]
    fn a_function_that_only_a_table_after_a_call_that_never_returns_reaches_is_counted() {
        // The Reviewer's plant: call a function that never returns (int3); a table of two
        // relocated entries; a function that only the table reaches, with push site.
        let function = 13;
        let never = 19;
        let text = code(&[
            &[0xE8],
            &le(never - 5),
            &le(va(function)),
            &le(va(function)),
            &[0x68],
            &le(DTOR_SITE),
            &[0xC3, 0xCC],
        ]);
        let (found, reach) = operands(&made_up_pe(&text, &[], &[5, 9]), only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Push)), Some(&1), "{reach:?}");
        assert!(
            reach.dropped >= 1,
            "the fall-through into the table was not dropped: {reach:?}"
        );
    }

    #[test]
    fn a_relocated_immediate_that_points_at_a_table_drops_its_block_and_the_site_stays() {
        // push offset table; ret; the table; the function of its entries, with push site.
        let table = 6;
        let function = 14;
        let text = code(&[
            &[0x68],
            &le(va(table)),
            &[0xC3],
            &le(va(function)),
            &le(va(function)),
            &[0x68],
            &le(DTOR_SITE),
            &[0xC3],
        ]);
        let (found, reach) = operands(&made_up_pe(&text, &[], &[1, table, table + 4]), only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Push)), Some(&1), "{reach:?}");
        assert!(reach.dropped >= 1, "{reach:?}");
    }

    #[test]
    fn a_site_behind_a_garbage_jump_after_a_call_that_never_returns_is_unreached() {
        // The Reviewer's plant (verdict 39): call a function that never returns (int3 at 17);
        // after it, bytes with no relocation that decode as jmp to 10; from 10 the bytes decode
        // as mov eax, imm32 over the push of the real function at 11 that nothing reaches. The
        // site lies inside that mov: it must count, never be removed.
        let text = code(&[
            &[0xE8],
            &le(12),
            &[0xE9],
            &le(0),
            &[0xB8, 0x68],
            &le(DTOR_SITE),
            &[0xC3, 0xCC],
        ]);
        let (found, reach) = operands(&made_up_pe(&text, &[], &[]), only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Unreached)), Some(&1), "{reach:?}");
        assert_eq!(reach.removed, 0, "{reach:?}");
        assert!(reach.weak > 0, "{reach:?}");
    }

    #[test]
    fn a_block_that_only_a_block_out_reaches_is_out() {
        // call 7, where the bytes from 7 are int3: the call never returns, so the mov from 5 is
        // weak; the strong block at 7 takes its bytes, and the mov's block is out. Its jmp to
        // 15 was the only way to the push there: that block is out too, and the site is unreached.
        let text = code(&[
            &[0xE8],
            &le(2),
            &[0xB8, 0x90, 0xCC, 0x90, 0x90, 0xE9],
            &le(0),
            &[0x68],
            &le(DTOR_SITE),
            &[0xC3],
        ]);
        let (found, reach) = operands(&made_up_pe(&text, &[], &[]), only(DTOR_SITE));
        assert_eq!(found.get(&(DTOR_SITE, Form::Unreached)), Some(&1), "{reach:?}");
        assert!(
            reach.overlaps.is_empty(),
            "a strong and a weak block are no overlap: {reach:?}"
        );
    }

    #[test]
    fn a_fall_through_is_strong_only_after_a_call_that_comes_back() {
        // call f; then call rel32 (00 68 F0 FE, a target outside the code), push 0, ret; f at 13.
        // The window F0 FE 6A 00 after the 0x68 is inside the second call.
        for (f, removed) in [(0xC3, 1), (0xCC, 0)] {
            let text = code(&[&[0xE8], &le(8), &[0xE8, 0x00, 0x68, 0xF0, 0xFE, 0x6A, 0x00, 0xC3, f]]);
            let (found, reach) = operands(&made_up_pe(&text, &[], &[]), only(0x6AFEF0));
            assert_eq!(reach.removed, removed, "f {f:#x}: {reach:?}");
            // After a call that never returns, the window counts by the byte before it.
            let unreached = found.get(&(0x6AFEF0, Form::Unreached)).copied().unwrap_or(0);
            assert_eq!(unreached, 1 - removed, "f {f:#x}: {found:?}");
        }
    }

    #[test]
    fn padding_before_a_push_0_is_no_operand() {
        // ret; int3 padding; push 0; ret: the window CC CC 6A 00 is preceded by CC, which no
        // instruction of the three ends in. The line 0x6acccc of the old baseline.
        let text = [0xC3, 0xCC, 0xCC, 0xCC, 0xCC, 0x6A, 0x00, 0xC3];
        let (found, _) = operands(&made_up_pe(&text, &[], &[]), only(0x006A_CCCC));
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn two_strong_starts_that_disagree_are_both_out_and_listed() {
        // je +1 to the byte 3; mov eax, 0xC3 from the byte 2; ret: the bytes 2 to 6 decode
        // as a mov from the entry and as a ret from the branch target.
        let text = [0x74, 0x01, 0xB8, 0xC3, 0x00, 0x00, 0x00, 0xC3];
        let pe = made_up_pe(&text, &[], &[]);
        let (_, reach) = operands(&pe, only(DTOR_SITE));
        assert_eq!(reach.overlaps, vec![va(3)], "{reach:?}");
        assert!(check(&pe, &[], followed).failures.is_empty());
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
        let text = code(&[&[0x68], &le(DTOR_SITE), &[0xC3]]);
        let pe = made_up_pe(&text, &[], &[]);
        let report = check(&pe, &[], followed);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        let report = check(&pe, &[], |value| value != DTOR_SITE && followed(value));
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].contains("0x6a9d83, NiDX8Renderer::deleting_dtor+0x3"),
            "{}",
            report.failures[0]
        );
        assert!(
            report.failures[0].contains("in the stolen bytes of NiDX8Renderer::deleting_dtor"),
            "{}",
            report.failures[0]
        );
    }

    #[test]
    fn an_entry_passes_only_by_its_form_and_count_in_the_baseline() {
        // `NiDX8Renderer::dtor` 0x6A9E10: MWSE names it as the expected callee of its call.
        let push: &[u8] = &code(&[&[0x68], &le(0x6A9E10)]);
        let pe = made_up_pe(&code(&[push, &[0xC3]]), &[], &[]);
        assert_eq!(check(&pe, &[], followed).failures.len(), 1);
        let reviewed = Reviewed {
            value: 0x6A9E10,
            form: Form::Push,
            count: 1,
        };
        assert!(check(&pe, &[reviewed.clone()], followed).failures.is_empty());
        // The same value in another form, or one more time, is a new occurrence.
        let mov = Reviewed {
            form: Form::MovReg,
            ..reviewed.clone()
        };
        assert_eq!(check(&pe, &[mov], followed).failures.len(), 1);
        let twice = made_up_pe(&code(&[push, push, &[0xC3]]), &[], &[]);
        assert_eq!(check(&twice, &[reviewed], followed).failures.len(), 1);
    }

    #[test]
    fn a_count_lock_site_in_mwses_table_passes_and_other_values_fail() {
        // 0x6F3F5E: a count lock site in the body of the port of NiGeometry::pushStateToRenderer.
        const SITE: u32 = 0x6F3F5E;
        let func = function_of(SITE).unwrap();
        assert_eq!(func.name, "NiGeometry::pushStateToRenderer");
        assert_eq!(place(func, SITE).0, Place::Body);
        assert!(!followed(SITE) && !ports::wraps_engine(func.ea));

        let table = made_up_pe(&[0xC3], &le(SITE), &[]);
        let report = check(&table, &[], followed);
        assert!(report.failures.is_empty(), "{:?}", report.failures);

        // The null: a value of the same body that is no site fails as before.
        let other = made_up_pe(&[0xC3], &le(SITE + 2), &[]);
        assert_eq!(check(&other, &[], followed).failures.len(), 1);
        // The site's address as an operand of code is another patch: it fails.
        let push = made_up_pe(&code(&[&[0x68], &le(SITE), &[0xC3]]), &[], &[]);
        assert_eq!(check(&push, &[], followed).failures.len(), 1);
    }

    #[test]
    fn a_baseline_line_needs_a_reason_and_a_missing_value_is_stale() {
        assert!(parse_baseline("0x6a9e10 push 2").is_err());
        let lines = parse_baseline("# a comment\n0x6a9e10 push 2 the expected callee of the call at 0x6A9D83\n").unwrap();
        assert_eq!(
            lines,
            vec![Reviewed {
                value: 0x6A9E10,
                form: Form::Push,
                count: 2
            }]
        );
        assert_eq!(
            parse_baseline("0x6a9e10 unreached 1 a table").unwrap()[0].form,
            Form::Unreached
        );
        let report = check(&made_up_pe(&[0x90, 0x90, 0xC3], &[], &[]), &lines, followed);
        assert_eq!(report.stale.len(), 1);
        assert!(report.failures.is_empty());
    }

    /// The baseline of the repository.
    fn baseline() -> Vec<Reviewed> {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/mwse_sites.baseline"))
            .expect("tools/mwse_sites.baseline");
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
        let started = std::time::Instant::now();
        let report = check(&pe, &baseline(), followed);
        for line in report.lines.iter().chain(&report.stale) {
            println!("{line}");
        }
        println!("{}", report.reach.line());
        println!(
            "{path}: {} places in ported or census functions, {} failed, {} stale baseline lines; {:.1} s",
            report.lines.len(),
            report.failures.len(),
            report.stale.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(
            report.failures.is_empty(),
            "{} places of MWSE.dll are not followed:\n{}",
            report.failures.len(),
            report.failures.join("\n")
        );
    }

    /// The null test of the decode on a real DLL (`NIRENDER_MWSE_DLL`, the DLL of 2026-11-09,
    /// md5 c4b1ff6c): its site 0x6A9D83 is a reached instruction's operand, and the check fails
    /// on it without the follow. Skipped without the variable or when the DLL has no such site.
    #[test]
    fn the_dtor_site_of_a_real_dll_is_reached() {
        let Ok(path) = std::env::var("NIRENDER_MWSE_DLL") else {
            eprintln!("skipped: no NIRENDER_MWSE_DLL");
            return;
        };
        let pe = std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let (found, _) = operands(&pe, only(DTOR_SITE));
        println!("{path}: 0x6a9d83 {found:?}");
        if found.is_empty() {
            eprintln!("skipped: {path} names no 0x6a9d83");
            return;
        }
        assert!(
            found
                .keys()
                .all(|&(_, form)| matches!(form, Form::Push | Form::MovReg | Form::MovMem)),
            "{found:?}"
        );
        let report = check(&pe, &baseline(), |value| value != DTOR_SITE && followed(value));
        assert!(
            report
                .failures
                .iter()
                .any(|line| line.contains("0x6a9d83, NiDX8Renderer::deleting_dtor+0x3")),
            "{:?}",
            report.failures
        );
    }
}
