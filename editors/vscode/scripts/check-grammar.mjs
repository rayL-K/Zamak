// 校验 VS Code 扩展的声明式配置：JSON 可解析、TextMate 正则可编译、各文件引用一致。
// 用法：node editors/vscode/scripts/check-grammar.mjs
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url)); // editors/vscode/
const files = {
  pkg: `${root}package.json`,
  config: `${root}language-configuration.json`,
  grammar: `${root}syntaxes/zamak.tmLanguage.json`,
};
const [pkg, config, grammar] = Object.values(files).map((file) => JSON.parse(readFileSync(file, 'utf8')));

let regexes = 0;
const errors = [];
const walk = (node) => {
  if (Array.isArray(node)) return node.forEach(walk);
  if (!node || typeof node !== 'object') return;
  for (const [key, value] of Object.entries(node)) {
    if (['match', 'begin', 'end'].includes(key) && typeof value === 'string') {
      regexes += 1;
      try {
        new RegExp(value, 'm');
      } catch (error) {
        errors.push(`${key} /${value}/ -> ${error.message}`);
      }
    } else {
      walk(value);
    }
  }
};
walk(grammar);

const languages = pkg.contributes?.languages ?? [];
const grammars = pkg.contributes?.grammars ?? [];
if (languages.length !== 1) errors.push(`expected exactly one language contribution, got ${languages.length}`);
if (languages[0]?.id !== 'zamak') errors.push(`language id must be zamak, got ${languages[0]?.id}`);
if (!(languages[0]?.extensions ?? []).includes('.zm')) errors.push('language extensions must include .zm');
if (languages[0]?.configuration !== './language-configuration.json') errors.push('language configuration path missing');
if (grammars.length !== 1) errors.push(`expected exactly one grammar contribution, got ${grammars.length}`);
if (grammars[0]?.scopeName !== grammar.scopeName) errors.push(`grammar scopeName ${grammars[0]?.scopeName} != ${grammar.scopeName}`);
if (grammars[0]?.path !== './syntaxes/zamak.tmLanguage.json') errors.push('grammar path missing');
if (grammar.scopeName !== 'source.zamak') errors.push(`scopeName must be source.zamak, got ${grammar.scopeName}`);
if ((grammar.patterns ?? []).length === 0) errors.push('grammar has no top-level patterns');
for (const include of (grammar.patterns ?? []).map((pattern) => pattern.include).filter(Boolean)) {
  if (!grammar.repository[include.slice(1)]) errors.push(`missing repository entry for ${include}`);
}
for (const group of Object.keys(grammar.repository)) {
  if (!(grammar.patterns ?? []).some((pattern) => pattern.include === `#${group}`)) {
    errors.push(`repository group #${group} is never included`);
  }
}
const keywords = ['catch', 'fail', 'struct', 'enum', 'match', 'elif', 'pass', 'and', 'or', 'not', 'i64', 'f64', 'string', 'println', 'while', 'for', 'pub'];
const repository = JSON.stringify(grammar.repository);
for (const keyword of keywords) {
  if (!repository.includes(keyword)) errors.push(`grammar does not mention ${keyword}`);
}
if (config.comments?.blockComment) errors.push('block comments are not part of the language');
if (config.comments?.lineComment !== '#') errors.push(`lineComment must be #, got ${config.comments?.lineComment}`);
// 命令：extension.js 里注册的每个命令都要在 package.json 里声明，否则命令面板里找不到它
// （一键安装在状态栏和提示框里都用到）。注册时常用常量作命令名，所以先把常量解析成字面量。
const extension = readFileSync(`${root}extension.js`, 'utf8');
const constants = new Map();
for (const match of extension.matchAll(/const\s+([A-Z_]+)\s*=\s*\[\s*'([^']+)'/g)) {
  constants.set(`${match[1]}[0]`, match[2]);
}
const declared = new Set((pkg.contributes?.commands ?? []).map((entry) => entry.command));
const registered = new Set();
for (const match of extension.matchAll(/registerCommand\(\s*([^,]+?),/g)) {
  const target = match[1].trim();
  const literal = target.startsWith("'") ? target.slice(1, -1) : constants.get(target);
  if (literal) registered.add(literal);
  else errors.push(`cannot resolve the command name in registerCommand(${target})`);
}
for (const command of registered) {
  if (!declared.has(command)) errors.push(`command ${command} is registered but not declared in package.json`);
}
for (const command of declared) {
  if (!registered.has(command)) errors.push(`command ${command} is declared but never registered`);
}
if (!declared.has('zamak.install')) errors.push('missing the one-click install command');
// 代码块靠缩进：冒号结尾的行之后要自动缩进，折叠按缩进（offSide）。
if (config.folding?.offSide !== true) errors.push('folding.offSide must be true for an indentation-based language');
for (const [name, pattern] of [
  ['increaseIndentPattern', config.indentationRules?.increaseIndentPattern],
  ['decreaseIndentPattern', config.indentationRules?.decreaseIndentPattern],
]) {
  try {
    new RegExp(pattern);
  } catch (error) {
    errors.push(`${name} /${pattern}/ -> ${error.message}`);
  }
}
const increase = new RegExp(config.indentationRules?.increaseIndentPattern ?? '$^');
for (const line of ['fn main():', '    if n < 1:  # note', 'struct Point:', '        Circle(r):']) {
  if (!increase.test(line)) errors.push(`increaseIndentPattern should match ${JSON.stringify(line)}`);
}
for (const line of ['    let x: i64 = 1', '    # note:']) {
  if (increase.test(line)) errors.push(`increaseIndentPattern should not match ${JSON.stringify(line)}`);
}

console.log(`json files parsed: ${Object.keys(files).length}, regexes compiled: ${regexes}, repository groups: ${Object.keys(grammar.repository).length}`);
if (errors.length) {
  console.error(errors.join('\n'));
  process.exit(1);
}
console.log('vscode extension contribution graph OK');
