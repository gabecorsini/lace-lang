"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const vscode_1 = require("vscode");
const node_1 = require("vscode-languageclient/node");
let client;
function activate(context) {
    const serverOptions = {
        command: "lace",
        args: ["lsp"],
        transport: node_1.TransportKind.stdio,
    };
    const clientOptions = {
        documentSelector: [{ scheme: "file", language: "lace" }],
        synchronize: {
            fileEvents: vscode_1.workspace.createFileSystemWatcher("**/*.lace"),
        },
    };
    client = new node_1.LanguageClient("laceLsp", "Lace Language Server", serverOptions, clientOptions);
    client.start();
}
function deactivate() {
    if (!client)
        return undefined;
    return client.stop();
}
//# sourceMappingURL=extension.js.map