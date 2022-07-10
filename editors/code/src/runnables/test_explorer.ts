import * as vscode from 'vscode';
import * as path from 'path';
import { Ctx } from '../ctx';
import * as ra from '../lsp_ext';
import { RunStatusUpdate, RunStatusUpdateKind } from '../lsp_ext';
import { LanguageClient } from 'vscode-languageclient/node';
import { TestRunControler } from "./run_controler";
import * as tree from './tree_view';

const iconsRootPath = path.join(path.dirname(__dirname), '..', 'resources', 'icons');

function getIconUri(iconName: string, theme: string): vscode.Uri {
    return vscode.Uri.file(path.join(iconsRootPath, theme, `${iconName}.svg`));
}

// class Session {
//     constructor() {
//         this.id = "0";
//         this.crates = [];
//     }
//     getChildren(): Crate[] {
//         return this.crates;
//     }
// }

// class Crate extends vscode.TreeItem {
//     constructor(
//         id: string,
//         name: string,
//         modules: Module[],
//         location: string,
//     ) {
//         super(name, vscode.TreeItemCollapsibleState.Collapsed);
//         this.tooltip = location;
//         this.id = id;
//         this.modules = modules;
//     }

//     iconPath = {
//         light: getIconUri('squares', 'dark'),
//         dark: getIconUri('squares', 'dark'),
//     };

//     getChildren(): Module[] {
//         return this.modules;
//     }
// }

// class Module extends vscode.TreeItem {
//     constructor(
//         id: string,
//         name: string,
//         location: string,
//         modules?: Module[],
//         targets?: Function[],
//     ) {
//         super(name, vscode.TreeItemCollapsibleState.Collapsed);
//         this.location = location;
//         this.modules = modules;
//         this.targets = targets;
//         this.id = id;
//     }

//     iconPath = {
//         light: getIconUri('squares', 'dark'),
//         dark: getIconUri('squares', 'dark'),
//     };

//     getChildren(): (Function | Module)[] {
//         var res: (Function | Module)[] = [];
//         if (this.targets != undefined) {
//             res.push(...this.targets);
//         }
//         if (this.modules != undefined) {
//             res.push(...this.modules);
//         }
//         return res;
//     }
// }

// class Function extends vscode.TreeItem {
//     constructor(
//         id: string,
//         name: string,
//         location: string,
//         kind: TestKind,
//     ) {
//         super(name, vscode.TreeItemCollapsibleState.None);
//         this.location = location;
//         this.id = id;

//         switch (kind) {
//             case TestKind.Bench: {
//                 this.iconPath = {
//                     light: getIconUri('accelerator', 'dark'),
//                     dark: getIconUri('accelerator', 'dark'),
//                 };
//                 break;
//             }
//             case TestKind.Test: {
//                 this.iconPath = {
//                     light: getIconUri('test_sheet', 'dark'),
//                     dark: getIconUri('test_sheet', 'dark'),
//                 };
//                 break;
//             }
//         }
//     }

//     getChildren(): null {
//         return null;
//     }
// }

export class TestExplorerProvider {
    private controller: vscode.TestController;
    private testExecutor: TestRunControler;
    private runProfile: vscode.TestRunProfile;
    private debugProfile: vscode.TestRunProfile;

    /// Crawls the test's tree and find node's field that contain item with passed id.
    findItem(id: string): [vscode.TestItem, vscode.TestItemCollection] | null {
        const buff: vscode.TestItem[] = [];
        this.controller.items.forEach(i => buff.push(i));
        let holder = null;
        let target = null;
        while (!holder && buff.length != 0) {
            const current = buff.pop()!;
            current.children.forEach((item, collection) => {
                if (item.id == id) {
                    holder = collection;
                    target = item;
                    return;
                }
                buff.push(item);
            });
        }

        if (holder != null && target != null) {
            return [target, holder];
        }

        return null;
    }

    bfs(process: (parentField: vscode.TestItemCollection, node: vscode.TestItem | undefined) => void) {
        process(this.controller.items, undefined);

        const queue: Array<vscode.TestItemCollection> = [];
        if (this.controller.items.size == 0) {
            return;
        }
        queue.push(this.controller.items);
    
        while (queue.length != 0) {
            const current = queue.pop()!;
            current.forEach((item) => {
                process(current, item);
                if (item.children.size > 0) {
                    queue.push(item.children);
                }
            });
        }
    }

    applyUpdate(deltaUpdate: tree.DeltaUpdate) {
        function findAndRemove<T extends {targetId: number}>(obj: T[], pred: (value: T) => boolean): T | undefined {
            let index = obj.findIndex(pred);
            if (index) {
                return obj.splice(index, 1)[0];
            }
            return undefined;
        }
    
        function popIf<T>(obj: T[], pred: (value: T) => boolean): T | undefined {
            let item = obj.at(-1); 
            if (item !== undefined) {
                if (pred(item)) {
                    return obj.pop();
                } else {
                    return undefined;
                }
            }
            return undefined;
        }
    
        this.bfs((parentField, node) => {
            let targetId: string;
            if(node == undefined) {
                targetId = "0";
            } else {
                targetId = node.id;
            }

            const pred = (item: { targetId: number; }) => item.targetId == Number(targetId);

            let update = findAndRemove(deltaUpdate.update, pred);
            if (update !== undefined) {
                // node.label = patch.payload.name!;
                // TODO: WHYYYYYYYY??????
                // node.uri = vscode.Uri.file(patch.payload.location!);
            }

            let patch = findAndRemove(deltaUpdate.delete, pred);
            if (patch !== undefined) {
                parentField.delete(targetId);
            }
            
            let append = popIf(deltaUpdate.append, pred);
            if (append !== undefined) {
                let apended = append.item;
                let item = this.controller.createTestItem(apended.id, apended.name);
                parentField.add(item);
            }
        });
    }

    handleRunRequest(request: vscode.TestRunRequest, token: vscode.CancellationToken) {
        // //TODO: token.onCancellationRequested(() => this.testExecutor.cancel());

        // const run = this.controller.createTestRun(request, undefined, true);

        // const queue: vscode.TestItem[] = [];
        // if (request.include) {
        //     request.include.forEach(test => queue.push(test));
        // } else {
        //     this.controller.items.forEach(test => queue.push(test));
        // }

        // bfsTestItems(queue, (test, context) => {
        //     context.isTerminate = token.isCancellationRequested;

        //     if (request.exclude?.includes(test)) {
        //         context.isSkipping = true;
        //     } else {
        //         run.enqueued(test);
        //     }
        // });

        // this.testExecutor.onStatusUpdate((updates) => {
        //     for (const update of updates) {
        //         switch (update.kind) {
        //             case RunStatusUpdateKind.RawOutput: {
        //                 run.appendOutput(update.message);
        //                 break;
        //             }
        //             case RunStatusUpdateKind.Skiped: {
        //                 const [item, _] = this.findItem(update.id)!;
        //                 run.skipped(item);
        //                 break;
        //             }
        //             case RunStatusUpdateKind.Errored: {
        //                 const [item, _] = this.findItem(update.id)!;
        //                 run.errored(item, update.message, update.duration);
        //                 break;
        //             }
        //             case RunStatusUpdateKind.Failed: {
        //                 const [item, _] = this.findItem(update.id)!;
        //                 run.failed(item, update.message, update.duration);
        //                 break;
        //             }
        //             case RunStatusUpdateKind.Passed: {
        //                 const [item, _] = this.findItem(update.id)!;
        //                 run.passed(item, update.duration);
        //                 break;
        //             }
        //             case RunStatusUpdateKind.Started: {
        //                 const [item, _] = this.findItem(update.id)!;
        //                 run.started(item);
        //                 break;
        //             }
        //             case RunStatusUpdateKind.Finish: {
        //                 run.end();
        //                 break;
        //             }
        //         }
        //     }
        // });

        // const kind: ra.RunKind = (() => {
        //     switch (request.profile) {
        //         case this.runProfile:
        //             return ra.RunKind.Run;
        //         case this.debugProfile:
        //             return ra.RunKind.Debug;
        //         default:
        //             return undefined;
        //     }
        // })()!;

        // const includedIds = request.include?.map(i => i.id);
        // const excludeIds = request.exclude?.map(i => i.id);
        // this.testExecutor.execute(includedIds, excludeIds, kind);
    }

    /// Create TestController, set onDidChangeTreeData notified listener function,
    /// create two profile for usually run and debug 
    constructor(client: LanguageClient) {
        this.testExecutor = new TestRunControler(client);
        this.controller = vscode.tests.createTestController("rust-analyzer", "rust");
                
        client.onNotification(ra.dataUpdate, (params) => {
            this.applyUpdate(params);
        })
        
        this.runProfile = this.controller.createRunProfile(
            "Usually run",
            vscode.TestRunProfileKind.Run,
            (request, token) => this.handleRunRequest(request, token),
            true
        );

        this.debugProfile = this.controller.createRunProfile(
            "Usually debug",
            vscode.TestRunProfileKind.Debug,
            (request, token) => this.handleRunRequest(request, token),
            true
        );
    }
}






