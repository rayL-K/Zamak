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
const keywords = ['catch', 'fail', 'struct', 'i64', 'string', 'println', 'while', 'pub'];
const repository = JSON.stringify(grammar.repository);
for (const keyword of keywords) {
  if (!repository.includes(keyword)) errors.push(`grammar does not mention ${keyword}`);
}
if (config.comments?.blockComment) errors.push('block comments are not part of the language');
if (config.comments?.lineComment !== '//') errors.push(`lineComment must be //, got ${config.comments?.lineComment}`);

console.log(`json files parsed: ${Object.keys(files).length}, regexes compiled: ${regexes}, repository groups: ${Object.keys(grammar.repository).length}`);
if (errors.length) {
  console.error(errors.join('\n'));
  process.exit(1);
}
console.log('vscode extension contribution graph OK');
