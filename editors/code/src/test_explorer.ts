import * as vscode from 'vscode';
import * as path from 'path';
import { Ctx } from './ctx';
import * as ra from './lsp_ext';
import { RunStatusUpdate, RunStatusUpdateKind } from './lsp_ext';

const iconsRootPath = path.join(path.dirname(__dirname), '..', 'resources', 'icons');

function getIconUri(iconName: string, theme: string): vscode.Uri {
    return vscode.Uri.file(path.join(iconsRootPath, theme, `${iconName}.svg`));
}

interface Session {
    kind: NodeKind.Session;
    id: "0",
    crates: Crate[];
}

enum NodeKind {
    Session = "Session",
    Crate = "Crate",
    Module = "Module",
    Function = "Function",
}

interface Crate {
    kind: NodeKind.Crate;
    id: string;
    name: string;
    modules: Module[];
    location: string;
}

interface Module {
    kind: NodeKind.Module;
    id: string;
    name: string;
    modules?: Module[];
    targets?: Function[];
    location: string;
}

enum TestKind {
    Test,
    Bench,
    Bin,
}

interface Function {
    kind: NodeKind.Function;
    id: string;
    name: string;
    location: string;
    range: [[number, number], [number, number]];
    testKind: TestKind;
}

/// The view synchronized with RA data by `DeltaUpdate`'s. The update is an array   
/// of elementary actions called a `Patch`. After applying an update to the tree 
/// it will become synchronized.
///
/// All groups are transitive among themselves, in addition the Update and Delete 
/// patches are transitive in a group, but Append is not transitive in a group 
/// and must be applied in order

interface DeltaUpdate {
    id: number,
    delete: Delete[],
    update: Update[],
    append: Append[]
}

interface Delete {
    targetId: number;
}

interface Update {
    targetId: number;
    payload: {
        name?: string;
        location?: string;
        testKind?: TestKind;
    };
}

type AppendItem = Crate | Module | Function;
interface Append {
    targetId: number;
    item: AppendItem;
}

class Session {
    getChildren(): Crate[] {
        return this.crates;
    }
}

class Crate extends vscode.TreeItem {
    constructor(
        id: string,
        name: string,
        modules: Module[],
        location: string,
    ) {
        super(name, vscode.TreeItemCollapsibleState.Collapsed);
        this.tooltip = location;
        this.id = id;
        this.modules = modules;
    }

    iconPath = {
        light: getIconUri('squares', 'dark'),
        dark: getIconUri('squares', 'dark'),
    };

    getChildren(): Module[] {
        return this.modules;
    }
}

class Module extends vscode.TreeItem {
    constructor(
        id: string,
        name: string,
        location: string,
        modules?: Module[],
        targets?: Function[],
    ) {
        super(name, vscode.TreeItemCollapsibleState.Collapsed);
        this.location = location;
        this.modules = modules;
        this.targets = targets;
        this.id = id;
    }

    iconPath = {
        light: getIconUri('squares', 'dark'),
        dark: getIconUri('squares', 'dark'),
    };

    getChildren(): (Function | Module)[] {
        var res: (Function | Module)[] = [];
        if (this.targets != undefined) {
            res.push(...this.targets);
        }
        if (this.modules != undefined) {
            res.push(...this.modules);
        }
        return res;
    }
}

class Function extends vscode.TreeItem {
    constructor(
        id: string,
        name: string,
        location: string,
        kind: TestKind,
    ) {
        super(name, vscode.TreeItemCollapsibleState.None);
        this.location = location;
        this.id = id;

        switch (kind) {
            case TestKind.Bench: {
                this.iconPath = {
                    light: getIconUri('accelerator', 'dark'),
                    dark: getIconUri('accelerator', 'dark'),
                };
                break;
            }
            case TestKind.Test: {
                this.iconPath = {
                    light: getIconUri('test_sheet', 'dark'),
                    dark: getIconUri('test_sheet', 'dark'),
                };
                break;
            }
        }
    }

    getChildren(): null {
        return null;
    }
}

type Node = Session | Crate | Module | Function;

function bfs(root: Node, process: (parentField: Node[], node: Node) => void) {
    const queue: Array<Node[]> = [];
    let childrens = root.getChildren();
    if (childrens != null) {
        queue.push(childrens);
    }
    while (queue.length != 0) {
        const current = queue.pop()!;
        for (let item of current) {
            process(current, item);
            
            let childrens = root.getChildren();
            if (childrens != null) {
                queue.push(childrens);
            }
        }
    }
}

interface BfsContext {
    isTerminate: boolean;
    isSkipping: boolean;
}

function bfsTestItems(root: vscode.TestItem[], process: (node: vscode.TestItem, context: BfsContext) => void) {
    const context: BfsContext = { isTerminate: false, isSkipping: false };
    const queue: Array<vscode.TestItem> = root;
    while (queue.length != 0 && !context.isTerminate) {
        const current = queue.pop()!;
        process(current, context);
        if (context.isSkipping) {
            context.isSkipping = false;
            continue;
        }
        current?.children.forEach(i => queue.push(i));
    }
}

export class TestDataProvider implements vscode.TreeDataProvider<Node> {
    private treeChangeEmitter: vscode.EventEmitter<Node | undefined> = new vscode.EventEmitter<Node | undefined>();
    readonly onDidChangeTreeData: vscode.Event<Node | undefined> = this.treeChangeEmitter.event;
    private tree: Session = new Session;

    getChildren(element?: Node): vscode.ProviderResult<Node[]> {
        if (element == undefined) {
            return Promise.resolve([this.tree]);
        }

        return element.getChildren();
    }

    getTreeItem(element: Node): Node {
        return element;
    }

    handleCreate(target: Session | Crate | Module, patch: Append) {
        switch (target.kind) {
            case NodeKind.Session: {
                if (patch.item.kind != NodeKind.Crate) {
                    throw Error(`${patch.item.kind} cant't be payload for ${NodeKind.Session} target`);
                }
                target.crates.push(patch.item);
            }
                break;
            case NodeKind.Crate: {
                if (patch.item.kind != NodeKind.Module) {
                    throw Error(`${patch.item.kind} cant't be payload for ${NodeKind.Crate} target`);
                }
                target.modules.push(patch.item);
            }
                break;
            case NodeKind.Module: {
                if (patch.item.kind == NodeKind.Module) {
                    if (target.modules == undefined) {
                        target.modules = [];
                    }
                    target.modules.push(patch.item);
                } else if (patch.item.kind == NodeKind.Function) {
                    if (target.modules == undefined) {
                        target.modules = [];
                    }
                    target.targets!.push(patch.item);
                } else {
                    throw Error(`${patch.item.kind} cant't be payload for ${NodeKind.Module} target`);
                }
            }
                break;
        }
    }

    handleDelete(node: Session | Crate | Module, parentField: Array<Node>) {
        const index = parentField.indexOf(node);
        parentField.splice(index, 1);
    }

    handleUpdate(node: Crate | Module | Function, patch: Update) {
        switch (node.kind) {
            case NodeKind.Crate: {
                node.location = patch.payload.location!;
                node.name = patch.payload.name!;
            }
                break;
            case NodeKind.Module: {
                node.name = patch.payload.name!;
                node.location = patch.payload.location!;
            }
                break;
            case NodeKind.Function: {
                node.name = patch.payload.name!;
                node.location = patch.payload.location!;
                node.testKind = patch.payload.testKind!;
            }
                break;
        }
    }

    public applyUpdate(deltaUpdate: DeltaUpdate) {
        function findAndRemove<T extends {targetId: number}>(obj: T[], pred: (value: T) => boolean): T | undefined {
            let index = obj.findIndex(pred);
            if (index) {
                return obj.splice(index, 1)[0];
            }
            return undefined;
        }

        function shiftIf<T>(obj: T[], pred: (value: T) => boolean): T | undefined {
            let item = obj[0];
            if (item !== undefined) {
                if (pred(item)) {
                    return obj.shift();
                } else {
                    return undefined;
                }
            }
            return undefined;
        }

        bfs(this.tree, (parentField, node) => {
            const pred = (item: { targetId: number; }) => item.targetId == Number(node.id);

            if (node.kind !== NodeKind.Session) {
                let update = findAndRemove(deltaUpdate.update, pred);
                if (update !== undefined) {
                    this.handleUpdate(node, update);

                    this.treeChangeEmitter.fire(node);
                }
            }
            
            if (node.kind !== NodeKind.Function) {
                let patch = findAndRemove(deltaUpdate.delete, pred);
                if (patch !== undefined) {
                    this.handleDelete(node, parentField);

                    this.treeChangeEmitter.fire(node);
                }
                
                let append = shiftIf(deltaUpdate.append, pred);
                if (append !== undefined) {
                    this.handleCreate(node, append);

                    this.treeChangeEmitter.fire(node);
                }
            }
        });
    }
}

/**
 * Provides an API for creation and control tests run, and receiving notification of its
 * state.
 */
class TestRunControler {
    private readonly client;
    private readonly emitter;

    constructor(ctx: Ctx) {
        this.client = ctx.client;
        this.emitter = new vscode.EventEmitter<RunStatusUpdate>();
        this.onStatusUpdate = this.emitter.event;
        this.client.onNotification(ra.runStatus, this.emitter.fire);
    }

    /**
     * Subscription function that accepts a callback that fires when an event occurs.
     */
    readonly onStatusUpdate: vscode.Event<RunStatusUpdate>;

    /**
     * Creates run from set of tests.
     * 
     * We can think about it like some funny branch algebra. Since the data are in sync, 
     * we represent a branch by its root as the backend can select branches by it. 
     * So, insted to pass all needed test for execution, we represented it by function:
     * 
     * launched = include / exclude
     * 
     * @param include Selectable branch roots from the test tree
     * @param exclude Substractable subbranch roots from included forest
     * @param runKind 
     */
    async execute(include: string[] | undefined, exclude: string[] | undefined, runKind: ra.RunKind) {
        this.client.sendRequest(ra.runTests, { include, exclude, runKind });
    }

    /**
     * Interrupts tests in current run
     */
    async cancel(exact: string[]) {
        this.client.sendRequest(ra.abortTests, { exact });
    }
}

export class TestExplorerProvider {
    private controller: vscode.TestController;
    private treeDataProvider: TestDataProvider;
    private testExecutor: TestRunControler;
    private runProfile: vscode.TestRunProfile;
    private debugProfile: vscode.TestRunProfile;

    /// Crawls the test's tree and find node's field that contain item with passed id.
    findItem(id: string): [vscode.TestItem, vscode.TestItemCollection] | null {
        const buff: vscode.TestItem[] = [];
        this.controller.items.forEach(i => buff.push(i));
        let holder = null;
        while (!holder && buff.length != 0) {
            const current = buff.pop()!;
            current.children.forEach((item, collection) => {
                if (item.id == id) {
                    holder = collection;
                }
                buff.push(item);
            });
        }
        return holder;
    }

    /// Maps Node to TestItem
    convert(node: Node) {
        const uri = vscode.Uri.file(node.location);
        return this.controller.createTestItem(node.id, node.label as string, uri);
    }

    async updateBranch(branchRoot: Node | void | null | undefined) {
        const queue: [Node, vscode.TestItem][] = [];
        if (branchRoot == undefined || branchRoot == null) {
            const childrens = await this.treeDataProvider.getChildren();
            if (childrens) {
                const binded: [Node, vscode.TestItem][] = [];
                for (const child of childrens) {
                    const item = this.convert(child);
                    binded.push([child, item]);
                }
                this.controller.items.replace(binded.map(i => i[1]));
                queue.push(...binded);
            }
        } else {
            const [_, collection] = this.findItem(branchRoot.id)!;
            const item = this.convert(branchRoot);
            collection.add(item);
            queue.push([branchRoot, item]);
        }

        while (queue.length != 0) {
            const current = queue.pop()!;
            const childrens = current[0].getChildren();
            if (childrens) {
                const binded: [Node, vscode.TestItem][] = [];
                for (const child of childrens) {
                    const item = this.convert(child);
                    binded.push([child, item]);
                }
                current[1].children.replace(binded.map(i => i[1]));
                queue.push(...binded);
            }
        }
    }

    handleRunRequest(request: vscode.TestRunRequest, token: vscode.CancellationToken) {
        //TODO: token.onCancellationRequested(() => this.testExecutor.cancel());

        const run = this.controller.createTestRun(request, undefined, true);

        const queue: vscode.TestItem[] = [];
        if (request.include) {
            request.include.forEach(test => queue.push(test));
        } else {
            this.controller.items.forEach(test => queue.push(test));
        }

        bfsTestItems(queue, (test, context) => {
            context.isTerminate = token.isCancellationRequested;

            if (request.exclude?.includes(test)) {
                context.isSkipping = true;
            } else {
                run.enqueued(test);
            }
        });

        this.testExecutor.onStatusUpdate((updates) => {
            for (const update of updates) {
                switch (update.kind) {
                    case RunStatusUpdateKind.RawOutput: {
                        run.appendOutput(update.message);
                        break;
                    }
                    case RunStatusUpdateKind.Skiped: {
                        const [item, _] = this.findItem(update.id)!;
                        run.skipped(item);
                        break;
                    }
                    case RunStatusUpdateKind.Errored: {
                        const [item, _] = this.findItem(update.id)!;
                        run.errored(item, update.message, update.duration);
                        break;
                    }
                    case RunStatusUpdateKind.Failed: {
                        const [item, _] = this.findItem(update.id)!;
                        run.failed(item, update.message, update.duration);
                        break;
                    }
                    case RunStatusUpdateKind.Passed: {
                        const [item, _] = this.findItem(update.id)!;
                        run.passed(item, update.duration);
                        break;
                    }
                    case RunStatusUpdateKind.Started: {
                        const [item, _] = this.findItem(update.id)!;
                        run.started(item);
                        break;
                    }
                    case RunStatusUpdateKind.Finish: {
                        run.end();
                        break;
                    }
                }
            }
        });

        const kind: ra.RunKind = (() => {
            switch (request.profile) {
                case this.runProfile:
                    return ra.RunKind.Run;
                case this.debugProfile:
                    return ra.RunKind.Debug;
                default:
                    return undefined;
            }
        })()!;

        const includedIds = request.include?.map(i => i.id);
        const excludeIds = request.exclude?.map(i => i.id);
        this.testExecutor.execute(includedIds, excludeIds, kind);
    }

    /// Create TestController, set onDidChangeTreeData notified listener function,
    /// create two profile for usually run and debug 
    constructor(ctx: Ctx) {
        this.testExecutor = new TestRunControler(ctx);
        this.controller = vscode.tests.createTestController("rust-analyzer", "rust");
        this.treeDataProvider = new TestDataProvider();
        this.treeDataProvider.onDidChangeTreeData!(this.updateBranch);
        ctx.client.onNotification(ra.dataUpdate, (params) => {
            this.treeDataProvider.applyUpdate(params);
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






