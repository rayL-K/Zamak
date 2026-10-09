#!/usr/bin/env node
// Zamak 构建成本基准程序生成器。
// 同一份规格 → 两套实现（Rust workspace / Go module），供 DESIGN.md 第 8 节对比用。
// 用法：node bench/gen.mjs --lines 5000
// 产物：bench/generated/medium-cli/{rust,go} + manifest.json（measure.ps1 的编辑锚点）

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const LINES = Number(process.argv.find(a => a.startsWith('--lines='))?.split('=')[1] ?? 5000);

// ---- 规格：10 个编译单元，主链 lex→core→util→model→store→query→render→cli + 旁支 calc、fmtx ----
const CHAIN = [
  { name: 'lex',    deps: [] },
  { name: 'core',   deps: ['lex'] },
  { name: 'util',   deps: ['core'] },
  { name: 'model',  deps: ['core', 'util'] },
  { name: 'calc',   deps: ['model'] },
  { name: 'store',  deps: ['model', 'util'] },
  { name: 'query',  deps: ['store', 'model'] },
  { name: 'fmtx',   deps: ['core'] },
  { name: 'render', deps: ['query', 'fmtx', 'model'] },
  { name: 'cli',    deps: ['render', 'query', 'util'] },
];

// 公开签名锚点的调用点所在单元（场景 6：改 core 公开签名 → 下游级联）
const SIG_CALLERS = ['util', 'model', 'store', 'query', 'render', 'cli'];

// 各单元的三方依赖
const RUST_EXTRA = {
  lex:    [],
  core:   ['serde', 'thiserror'],
  util:   [],
  model:  ['serde'],
  calc:   [],
  store:  ['serde', 'serde_json', 'thiserror'],
  query:  ['regex'],
  fmtx:   [],
  render: ['thiserror'],
  cli:    ['clap', 'anyhow'],
};
const RUST_VER = { serde: 'serde = { version = "1", features = ["derive"] }', clap: 'clap = { version = "4", features = ["derive"] }', thiserror: 'thiserror = "2"', serde_json: 'serde_json = "1"', regex: 'regex = "1"', anyhow: 'anyhow = "1"' };

const FILES_PER_UNIT = Math.max(3, Math.min(8, Math.round(LINES / 10 / 105)));
const FNS_PER_MODULE = 6;

let LINES_ACTUAL = 0;
const countLines = (s) => { LINES_ACTUAL += s.split('\n').length; return s; };
const hexConst = (u, m, f) => (((u + 1) * 0x9E3779B1 + (m + 1) * 0x85EBCA6B + (f + 1) * 0xC2B2AE35) >>> 0).toString(16).toUpperCase().padStart(8, '0');

// ================= Rust =================

function rustModuleFile(u, uName, m) {
  const mod = `m${m + 1}`;
  const f = (i) => `f_${mod}_${i}`;
  const usesSerde = RUST_EXTRA[uName].includes('serde');
  const usesThiserror = RUST_EXTRA[uName].includes('thiserror');
  let out = '';
  if (usesSerde) out += `use serde::{Serialize, Deserialize};\n`;
  if (usesThiserror) out += `use thiserror::Error;\n`;
  if (CHAIN[u].deps.length) out += `use zm_${CHAIN[u].deps[0]}::m1::f_m1_3 as dep_f_m1_3;\n`;
  if (uName === 'store') out += `use serde_json::json;\n`;
  if (uName === 'query') out += `use regex::Regex;\n`;
  out += `\n// 编译单元 ${uName} 模块 ${mod}\npub(crate) const TWEAK: u64 = 0; // BENCH_TWEAK\n\n`;
  const derive = usesSerde ? 'Serialize, Deserialize, Debug, Clone, Copy' : 'Debug, Clone, Copy';
  out += `#[derive(${derive})]\npub struct Rec_${mod} {\n    pub id: u64,\n    pub tag: u64,\n    pub bias: u64,\n}\n\n`;
  out += `#[derive(${derive.startsWith('Serialize') ? derive + ', PartialEq, Eq' : derive + ', PartialEq, Eq'})]\npub enum Shape_${mod} {\n    Flat,\n    Tilt(u64),\n    Bank(u64),\n}\n\n`;
  if (usesThiserror) out += `#[derive(Debug, Error)]\npub enum Err_${mod} {\n    #[error("${uName} reject {0}")]\n    Reject(u64),\n    #[error("${uName} parse: {0}")]\n    Parse(String),\n}\n\n`;
  for (let i = 1; i <= FNS_PER_MODULE; i++) {
    out += `pub fn ${f(i)}(input: u64, salt: u64) -> u64 {\n`;
    out += `    let base = TWEAK ^ 0x${hexConst(u, m, i)}u64;\n`;
    out += `    let mut acc = input ^ base ^ salt.wrapping_mul(11);\n`;
    out += `    for i in 0..5u64 {\n        acc = acc.wrapping_add(i.wrapping_mul(base.rotate_left((i % 7) as u32)));\n    }\n`;
    out += `    match acc & 3 {\n        0 => acc ^= salt,\n        1 => acc = acc.wrapping_mul(0x100000001b3),\n        2 => acc = acc.rotate_left(17).wrapping_sub(acc),\n        _ => acc = acc.wrapping_add(base),\n    }\n`;
    out += i > 1 ? `    acc ^ ${f(i - 1)}(acc ^ salt.wrapping_add(1), base)\n` : `    acc\n`;
    out += `}\n\n`;
  }
  out += `pub fn entry_${mod}(seed: u64) -> u64 {\n    let a = ${f(1)}(seed, seed.wrapping_add(3));\n    let b = ${f(FNS_PER_MODULE)}(a, a.rotate_left(9));\n`;
  out += CHAIN[u].deps.length ? `    b ^ dep_f_m1_3(b, a)\n` : `    b\n`;
  out += `}\n`;
  if (uName === 'store') out += `\npub fn encode_json(v: u64) -> String {\n    json!({ "v": v, "tag": TWEAK }).to_string()\n}\n`;
  if (uName === 'query') out += `\npub fn sniff(s: &str) -> u64 {\n    let re = Regex::new(r"\\d+").expect("valid");\n    re.find_iter(s).count() as u64\n}\n`;
  return countLines(out);
}

function rustLibRs(uName) {
  let out = `#![allow(dead_code)]\n\n`;
  for (let m = 0; m < FILES_PER_UNIT; m++) out += `pub mod m${m + 1};\n`;
  if (uName === 'core') {
    out += `\n// 公开签名锚点：场景 6（改公开签名触发下游级联）\npub fn sig_anchor(input: u64) -> u64 {\n    let base = input ^ 0x516A_C0DEu64;\n    base.rotate_left(13) ^ (base >> 7)\n}\n`;
  }
  if (SIG_CALLERS.includes(uName)) {
    out += `\n// 公开签名锚点调用点：场景 6\npub fn sig_call(input: u64) -> u64 {\n    zm_core::sig_anchor(input)\n}\n`;
  }
  return countLines(out);
}

function genRust() {
  const root = path.join(HERE, 'generated', 'medium-cli', 'rust');
  fs.rmSync(root, { recursive: true, force: true });
  fs.mkdirSync(root, { recursive: true });

  let ws = `[workspace]\nresolver = "2"\nmembers = [\n`;
  for (const c of CHAIN) ws += `    "${c.name}",\n`;
  ws += `]\n`;
  fs.writeFileSync(path.join(root, 'Cargo.toml'), countLines(ws));

  for (let u = 0; u < CHAIN.length; u++) {
    const c = CHAIN[u];
    fs.mkdirSync(path.join(root, c.name, 'src'), { recursive: true });

    let toml = `[package]\nname = "zm_${c.name}"\nedition = "2021"\n\n[dependencies]\n`;
    const depSet = new Set(c.deps);
    if (SIG_CALLERS.includes(c.name)) depSet.add('core');
    for (const d of depSet) toml += `zm_${d} = { path = "../${d}" }\n`;
    for (const dep of RUST_EXTRA[c.name]) toml += `${RUST_VER[dep]}\n`;
    if (c.name === 'cli') toml += `\n[[bin]]\nname = "medium-cli"\npath = "src/main.rs"\n\n[lib]\nname = "zm_cli_lib"\npath = "src/lib.rs"\n`;
    fs.writeFileSync(path.join(root, c.name, 'Cargo.toml'), countLines(toml));

    fs.writeFileSync(path.join(root, c.name, 'src', 'lib.rs'), rustLibRs(c.name));
    for (let m = 0; m < FILES_PER_UNIT; m++) {
      fs.writeFileSync(path.join(root, c.name, 'src', `m${m + 1}.rs`), rustModuleFile(u, c.name, m));
    }
  }

  // cli 库探针（main.rs 调用入口之一）
  const probe = `\n// 探针：main.rs 经由 lib 走进 query 链\npub fn probe(input: u64) -> u64 {\n    let mut acc = input ^ 0x9A11_0001u64;\n    for i in 0..4u64 {\n        acc = acc.wrapping_add(i.wrapping_mul(acc.rotate_left(5)));\n    }\n    acc ^ zm_query::m1::entry_m1(acc.rotate_left(2))\n}\n`;
  fs.appendFileSync(path.join(root, 'cli', 'src', 'lib.rs'), countLines(probe));

  fs.writeFileSync(path.join(root, 'cli', 'src', 'main.rs'), countLines(`use clap::Parser;

#[derive(Parser)]
#[command(name = "medium-cli", about = "Zamak 构建成本基准程序（Rust 侧）")]
struct Args {
    /// 起始种子
    seed: u64,
    /// 轮数
    #[arg(long, default_value_t = 3)]
    rounds: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut acc = args.seed;
    for _ in 0..args.rounds {
        acc = zm_render::m1::entry_m1(acc)
            ^ zm_query::m1::entry_m1(acc.rotate_left(3))
            ^ zm_cli_lib::probe(acc);
    }
    println!("{acc}");
    Ok(())
}
`));
  return root;
}

// ================= Go =================

function goModuleFile(u, uName, m) {
  const mod = `m${m + 1}`;
  const f = (i) => `F${m + 1}_${i}`;
  const first = m === 0;
  let out = `package ${uName}\n\n`;
  out += `import (\n\t"math/bits"\n`;
  if (first && CHAIN[u].deps.length) out += `\n\t"zambench/${CHAIN[u].deps[0]}"\n`;
  if (first && uName === 'store') out += `\t"encoding/json"\n`;
  if (first && uName === 'query') out += `\t"regexp"\n`;
  out += `)\n\n`;
  out += `// 编译单元 ${uName} 模块 ${mod}\nconst ${`tweak${mod}`} uint64 = 0 // BENCH_TWEAK\n\n`;
  out += `type Rec${mod} struct {\n\tID   uint64\n\tTag  uint64\n\tBias uint64\n}\n\n`;
  out += `type Shape${mod} uint8\n\nconst (\n\tShape${mod}Flat Shape${mod} = iota\n\tShape${mod}Tilt\n\tShape${mod}Bank\n)\n\n`;
  out += `func (s Shape${mod}) Name() string {\n\treturn [...]string{"flat", "tilt", "bank"}[s]\n}\n\n`;
  if (uName === 'core' || uName === 'store') {
    out += `type Err${mod} struct {\n\tKind string\n\tVal  uint64\n}\n\nfunc (e *Err${mod}) Error() string { return "${uName} " + e.Kind }\n\n`;
  }
  for (let i = 1; i <= FNS_PER_MODULE; i++) {
    out += `func ${f(i)}(input, salt uint64) uint64 {\n`;
    out += `\tbase := ${`tweak${mod}`} ^ 0x${hexConst(u, m, i)}\n`;
    out += `\tacc := input ^ base ^ salt*11\n`;
    out += `\tfor i := uint64(0); i < 5; i++ {\n\t\tacc += i * bits.RotateLeft64(base, int(i%7))\n\t}\n`;
    out += `\tswitch acc & 3 {\n\tcase 0:\n\t\tacc ^= salt\n\tcase 1:\n\t\tacc *= 0x100000001b3\n\tcase 2:\n\t\tacc = bits.RotateLeft64(acc, 17) - acc\n\tdefault:\n\t\tacc += base\n\t}\n`;
    out += i > 1 ? `\treturn acc ^ ${f(i - 1)}(acc^salt+1, base)\n` : `\treturn acc\n`;
    out += `}\n\n`;
  }
  if (first) {
    out += `func Run(seed uint64) uint64 {\n\ta := ${f(1)}(seed, seed+3)\n\tb := ${f(FNS_PER_MODULE)}(a, bits.RotateLeft64(a, 9))\n`;
    out += CHAIN[u].deps.length ? `\treturn b ^ ${CHAIN[u].deps[0]}.F1_3(b, a)\n` : `\treturn b\n`;
    out += `}\n`;
    if (uName === 'store') out += `\nfunc EncodeJSON(v uint64) string {\n\tb, _ := json.Marshal(struct {\n\t\tV   uint64 \`json:"v"\`\n\t\tTag uint64 \`json:"tag"\`\n\t}{v, tweakm1})\n\treturn string(b)\n}\n`;
    if (uName === 'query') out += `\nvar numRe = regexp.MustCompile(\`\\d+\`)\n\nfunc Sniff(s string) uint64 {\n\treturn uint64(len(numRe.FindAllString(s, -1)))\n}\n`;
  }
  return countLines(out);
}

function genGo() {
  const root = path.join(HERE, 'generated', 'medium-cli', 'go');
  fs.rmSync(root, { recursive: true, force: true });
  fs.mkdirSync(root, { recursive: true });

  fs.writeFileSync(path.join(root, 'go.mod'), countLines(`module zambench

go 1.27

require (
\tgithub.com/spf13/cobra v1.9.1
\tgithub.com/spf13/viper v1.19.0
\tgo.uber.org/zap v1.27.0
)
`));

  for (let u = 0; u < CHAIN.length; u++) {
    const c = CHAIN[u];
    fs.mkdirSync(path.join(root, c.name), { recursive: true });
    for (let m = 0; m < FILES_PER_UNIT; m++) {
      fs.writeFileSync(path.join(root, c.name, `m${m + 1}.go`), goModuleFile(u, c.name, m));
    }
    if (c.name === 'core') {
      fs.writeFileSync(path.join(root, 'core', 'lib.go'), countLines(`package core

import "math/bits"

// 公开签名锚点：场景 6（改公开签名触发下游级联）
func SigAnchor(input uint64) uint64 {
\tbase := input ^ 0x516AC0DE
\treturn bits.RotateLeft64(base, 13) ^ (base >> 7)
}
`));
    }
    if (SIG_CALLERS.includes(c.name)) {
      fs.writeFileSync(path.join(root, c.name, 'sig.go'), countLines(`package ${c.name}

import "zambench/core"

// 公开签名锚点调用点：场景 6
func SigCall(input uint64) uint64 { return core.SigAnchor(input) }
`));
    }
  }

  fs.writeFileSync(path.join(root, 'cli', 'cli.go'), countLines(`package cli

import (
\t"fmt"
\t"math/bits"

\t"github.com/spf13/cobra"
\t"github.com/spf13/viper"
\t"go.uber.org/zap"
\t"zambench/calc"
\t"zambench/core"
\t"zambench/fmtx"
\t"zambench/query"
\t"zambench/render"
\t"zambench/store"
)

func rol64(x uint64, n int) uint64 { return bits.RotateLeft64(x, n) }

var rootCmd = &cobra.Command{
\tUse:   "medium-cli",
\tShort: "Zamak 构建成本基准程序（Go 侧）",
\tRunE: func(cmd *cobra.Command, args []string) error {
\t\tseed := viper.GetUint64("seed")
\t\tlog, _ := zap.NewDevelopment()
\t\tdefer log.Sync()
\t\tvar acc uint64 = seed
\t\tfor i := uint64(0); i < 3; i++ {
\t\t\tacc = render.Run(acc) ^ query.Run(rol64(acc, 3))
\t\t\tacc = store.Run(acc) ^ calc.Run(rol64(acc, 5))
\t\t\tacc = fmtx.Run(acc) ^ core.SigAnchor(acc) ^ query.Sniff("a1b22")
\t\t\tlog.Info("round", zap.Uint64("acc", acc), zap.Uint64("i", i))
\t\t}
\t\tfmt.Println(store.EncodeJSON(acc))
\t\treturn nil
\t},
}

func Execute() {
\trootCmd.Flags().Uint64("seed", 42, "起始种子")
\tviper.SetDefault("seed", 42)
\t_ = viper.BindPFlag("seed", rootCmd.Flags().Lookup("seed"))
\tif err := rootCmd.Execute(); err != nil {
\t\tpanic(err)
\t}
}
`));

  fs.writeFileSync(path.join(root, 'main.go'), countLines(`package main

import "zambench/cli"

func main() {
\tcli.Execute()
}
`));
  return root;
}

// ================= 执行 =================

const rustRoot = genRust();
const goRoot = genGo();

const manifest = {
  generatedAt: new Date().toISOString(),
  spec: { lines: LINES, filesPerUnit: FILES_PER_UNIT, fnsPerModule: FNS_PER_MODULE },
  approxTotalSourceLines: LINES_ACTUAL,
  edits: {
    leaf:     { rust: 'cli/src/m1.rs',   go: 'cli/m1.go' },
    middle:   { rust: 'store/src/m1.rs', go: 'store/m1.go' },
    private:  { rust: 'core/src/m1.rs',  go: 'core/m1.go' },
    sigAnchor: {
      rust: 'core/src/lib.rs', go: 'core/lib.go',
      fromRust: 'pub fn sig_anchor(input: u64) -> u64 {',
      toRust:   'pub fn sig_anchor(input: u64, extra: u64) -> u64 {',
      fromGo: 'func SigAnchor(input uint64) uint64 {',
      toGo:   'func SigAnchor(input, extra uint64) uint64 {',
    },
    sigCallers: SIG_CALLERS.map(u => ({
      rust: `${u}/src/lib.rs`, go: `${u}/sig.go`,
      fromRust: 'zm_core::sig_anchor(input)', toRust: 'zm_core::sig_anchor(input, 0)',
      fromGo: 'core.SigAnchor(input)',        toGo: 'core.SigAnchor(input, 0)',
    })),
  },
};
fs.writeFileSync(path.join(HERE, 'generated', 'medium-cli', 'manifest.json'), JSON.stringify(manifest, null, 2));
console.log(`rust: ${rustRoot}\ngo:   ${goRoot}\napprox source lines: ${LINES_ACTUAL} (spec ${LINES}, files/unit ${FILES_PER_UNIT})`);
