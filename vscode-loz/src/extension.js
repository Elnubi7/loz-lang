const vscode = require("vscode");

let lozTerminal;

function activate(context) {
  context.subscriptions.push(
    vscode.window.onDidCloseTerminal((closedTerminal) => {
      if (closedTerminal === lozTerminal) {
        lozTerminal = undefined;
      }
    })
  );

  registerFileCommand(context, "loz.checkCurrentFile", "check");
  registerFileCommand(context, "loz.runCurrentFile", "run");
  registerFileCommand(context, "loz.buildCurrentFile", "build");
  registerFileCommand(context, "loz.generateLlvmIr", "llvm-ir");
  registerFileCommand(context, "loz.agentListCurrentFile", "agent list");
  registerFileCommand(context, "loz.workflowListCurrentFile", "workflow list");
}

function deactivate() {}

function registerFileCommand(context, commandId, subcommand) {
  const disposable = vscode.commands.registerCommand(commandId, () => {
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
      vscode.window.showErrorMessage("Open a .loz file to run Loz commands.");
      return;
    }

    const { document } = editor;
    if (document.isUntitled) {
      vscode.window.showErrorMessage("Save the current Loz file before running Loz commands.");
      return;
    }

    if (document.languageId !== "loz" && !document.fileName.endsWith(".loz")) {
      vscode.window.showErrorMessage("The active file is not recognized as a Loz source file.");
      return;
    }

    const workspaceFolder = vscode.workspace.getWorkspaceFolder(document.uri);
    const terminal = getTerminal(workspaceFolder && workspaceFolder.uri.fsPath);
    terminal.show(true);
    terminal.sendText(`loz ${subcommand} ${quotePath(document.uri.fsPath)}`);
  });

  context.subscriptions.push(disposable);
}

function getTerminal(cwd) {
  if (!lozTerminal || lozTerminal.exitStatus !== undefined) {
    const options = { name: "Loz" };
    if (cwd) {
      options.cwd = cwd;
    }
    lozTerminal = vscode.window.createTerminal(options);
  }
  return lozTerminal;
}

function quotePath(filePath) {
  return `"${filePath.replace(/(["\\$`])/g, "\\$1")}"`;
}

module.exports = {
  activate,
  deactivate,
};
