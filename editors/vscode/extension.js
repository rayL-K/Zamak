// Zamak for VS Code: starts `zam lsp` for diagnostics as you type, completion, hover,
// go to definition, and formatting. Syntax highlighting comes from the TextMate grammar
// and works even when `zam` isn't installed.
const vscode = require('vscode');
const { execFile } = require('child_process');
const { LanguageClient } = require('vscode-languageclient/node');

let client;
let status;
// 启动时问一次 `zam --version`；装好之后重新问。
let version;
// 同一次会话里只提示一次“没装 zam”，之后靠状态栏和命令重试。
let warnedMissing = false;

function serverLanguage() {
  // An explicit ZAMAK_LANG wins; otherwise follow the VS Code display language.
  if (process.env.ZAMAK_LANG) return process.env.ZAMAK_LANG;
  return vscode.env.language.toLowerCase().startsWith('zh') ? 'zh' : 'en';
}

function chinese() {
  return vscode.env.language.toLowerCase().startsWith('zh');
}

// 界面文字跟随 VS Code 的显示语言（与报错语言一致）。
function t(en, zh) {
  return chinese() ? zh : en;
}

function zamPath() {
  return vscode.workspace.getConfiguration('zamak').get('path') || 'zam';
}

// 问一次 `zam --version`；没装或命令跑不起来时返回 undefined。
function askVersion() {
  return new Promise((resolve) => {
    execFile(zamPath(), ['--version'], { timeout: 10000 }, (error, stdout) => {
      resolve(error ? undefined : stdout.toString().trim());
    });
  });
}

const INSTALL = ['zamak.install', '用 npm 安装', 'Install with npm'];
const INSTALLED = ['zamak.recheck', '已经装好了', 'I installed it'];

// 状态栏：装好了显示版本，没装显示一个可点的安装入口。
function updateStatus() {
  if (!status) return;
  if (version) {
    status.text = `$(check) zam ${version}`;
    status.tooltip = t(`Zamak ${version}`, `Zamak ${version}`);
    status.command = 'zamak.restartServer';
  } else {
    status.text = '$(warning) zam';
    status.tooltip = t(
      'Zamak is not installed. Click to install it with npm.',
      '没有找到 zam。点击用 npm 安装。',
    );
    status.command = INSTALL[0];
    status.backgroundColor = new vscode.ThemeColor(
      'statusBarItem.warningBackground',
    );
  }
  status.show();
}

// 一键安装：在终端里跑 `npm install -g zamak`，装好后再点一次确认。
function install() {
  const terminal = vscode.window.createTerminal(t('Install Zamak', '安装 Zamak'));
  terminal.show();
  terminal.sendText('npm install -g zamak', true);
  // 终端里的命令跑完拿不到退出码，所以请用户装完点一下确认。
  vscode.window
    .showInformationMessage(
      t(
        'Run the install command in the terminal, then confirm here to start the language server.',
        '在终端里执行安装命令，装好后在这里确认一下，就会启动语言服务器。',
      ),
      t(INSTALLED[2], INSTALLED[1]),
    )
    .then((choice) => {
      if (choice) vscode.commands.executeCommand(INSTALLED[0]);
    });
}

// 重新问版本；装上了就把语言服务器拉起来（之前因为没装而没启动）。
async function recheck() {
  version = await askVersion();
  updateStatus();
  if (!version) {
    vscode.window.showWarningMessage(
      t(
        'Zamak: still no "zam" on PATH. Check the terminal output, then try again. If it lives elsewhere, set "zamak.path".',
        'Zamak：还是找不到 zam。看看终端的输出再试一次；如果它装在别处，请在设置里改 "zamak.path"。',
      ),
    );
    return;
  }
  await start({ quiet: true });
}

async function start(options = {}) {
  if (client) return;
  const command = zamPath();
  client = new LanguageClient(
    'zamak',
    'Zamak',
    {
      command,
      args: ['lsp'],
      options: { env: { ...process.env, ZAMAK_LANG: serverLanguage() } },
    },
    { documentSelector: [{ scheme: 'file', language: 'zamak' }] },
  );
  try {
    await client.start();
    warnedMissing = false;
  } catch (error) {
    client = undefined;
    if (warnedMissing || options.quiet) return;
    warnedMissing = true;
    // 启动失败最常见的原因是没装 zam：给一次一键安装，而不是只指向设置。
    const choice = await vscode.window.showWarningMessage(
      t(
        `Zamak: couldn't start "${command} lsp". Install Zamak with npm, or set "zamak.path" if it lives elsewhere. Highlighting still works.`,
        `Zamak：无法启动“${command} lsp”。可以用 npm 安装 Zamak；装在别处就在设置里改 "zamak.path"。语法高亮照常可用。`,
      ),
      t(INSTALL[2], INSTALL[1]),
      'Open settings',
    );
    if (choice === t(INSTALL[2], INSTALL[1])) {
      install();
    } else if (choice) {
      vscode.commands.executeCommand('workbench.action.openSettings', 'zamak.path');
    }
  }
}

async function stop() {
  if (client) {
    const running = client;
    client = undefined;
    await running.stop();
  }
}

function activate(context) {
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  context.subscriptions.push(
    status,
    vscode.commands.registerCommand('zamak.restartServer', async () => {
      await stop();
      await start();
    }),
    vscode.commands.registerCommand(INSTALL[0], install),
    vscode.commands.registerCommand(INSTALLED[0], recheck),
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (event.affectsConfiguration('zamak.path')) {
        version = await askVersion();
        updateStatus();
        await stop();
        await start({ quiet: true });
      }
    }),
  );
  return (async () => {
    version = await askVersion();
    updateStatus();
    // 没装 zam 时不去启动（启动一定失败），只留状态栏上的安装入口；语法高亮照常可用。
    if (version) {
      await start();
      return;
    }
    const choice = await vscode.window.showInformationMessage(
      t(
        'Zamak: the zam compiler is not installed. Install it with npm to get diagnostics, completion, and formatting.',
        'Zamak：没有找到 zam 编译器。用 npm 安装后才能有报错提示、补全和格式化。',
      ),
      t(INSTALL[2], INSTALL[1]),
      t(INSTALLED[2], INSTALLED[1]),
    );
    if (choice === t(INSTALL[2], INSTALL[1])) install();
    else if (choice) vscode.commands.executeCommand(INSTALLED[0]);
  })();
}

function deactivate() {
  return stop();
}

module.exports = { activate, deactivate };
