import * as vscode from 'vscode';
import * as path from 'path';
import { Ctx } from './ctx';
import * as ra from './lsp_ext';
import { RunStatusUpdate, RunStatusUpdateKind } from './lsp_ext';

const iconsRootPath = path.join(path.dirname(__dirname), '..', 'resources', 'icons');

function getIconUri(iconName: string, theme: string): vscode.Uri {
    return vscode.Uri.file(path.join(iconsRootPath, theme, `${iconName}.svg`));
}

/// Runnable.
type Session = Iterable<Package>;

type Node = Package | Crate | Module | Function;

enum NodeKind {
    Package = "Package",
    Crate = "Crate",
    Module = "Module",
    Function = "Function",
}

interface Package {
    kind: NodeKind.Package;
    id: string;
    name: string;
    crates: Crate[];
    location: string;
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

type DeltaUpdate = Iterable<Patch>;

type Patch = Delete | Update | Create;

enum PatchKind {
    Delete = "DELETE",
    Update = "UPDATE",
    Create = "CREATE"
}

interface Delete {
    kind: PatchKind.Delete;
    targetId: string;
}

interface Update {
    kind: PatchKind.Update;
    targetId: string;
    payload: {
        name?: string;
        location?: string;
        testKind?: TestKind;
    };
}

interface Create {
    kind: PatchKind.Create;
    targetId: string;
    payload: Node;
}

class Package extends vscode.TreeItem {
    constructor(
        id: string,
        name: string,
        crates: Crate[],
        location: string,
    ) {
        super(name, vscode.TreeItemCollapsibleState.Collapsed);
        this.id = id;
        this.crates = crates;
        this.tooltip = location;
    }

    iconPath = {
        light: getIconUri('squares', 'dark'),
        dark: getIconUri('squares', 'dark'),
    };

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
            case TestKind.Unit: {
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

function bfs(root: Node, process: (parentField: Node[], node: Node) => void) {
    const queue: Array<Node> = [root];
    while (queue.length != 0) {
        const current = queue.pop();
        //@ts-ignore
        process(current);
        //@ts-ignore
        current.getChildren();
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
    private tree: Node;

    getChildren(element?: Node): vscode.ProviderResult<Node[]> {
        if (element == undefined) {
            return Promise.resolve([this.tree]);
        }

        return element.getChildren();
    }

    getTreeItem(element: Node): Node {
        return element;
    }

    // getParent(element: Node): Node {
    //
    // }

    constructor() {
        this.tree = new Module("test_id_0", "test_module_name", "/home/mrsmith/Desktop/Work/solstarter-ido/program/tests", undefined, [
            new Function("test_id_1", "test_function_name_1", "/home/mrsmith/Desktop/Work/solstarter-ido/program/tests", TestKind.Unit),
            new Function("test_id_2", "test_function_name_2", "/home/mrsmith/Desktop/Work/solstarter-ido/program/tests", TestKind.Unit),
            new Function("test_id_3", "test_function_name_3", "/home/mrsmith/Desktop/Work/solstarter-ido/program/tests", TestKind.Unit),
            new Function("test_id_4", "test_function_name_4", "/home/mrsmith/Desktop/Work/solstarter-ido/program/tests", TestKind.Unit),
            new Function("test_id_5", "blablabla", "/home/mrsmith/Desktop/Work/solstarter-ido/program/tests", TestKind.Unit),
        ]);
        this.treeChangeEmitter.fire(undefined);
    }
}

export class RunnableDataProvider {
    handleCreate(node: Node, patch: Create) {
        switch (node.kind) {
            case NodeKind.Package: {
                if (patch.payload.kind != NodeKind.Crate) {
                    throw Error(`${patch.payload.kind} cant't be payload for ${NodeKind.Package} target`);
                }
                node.crates.push(patch.payload);
            }
                break;
            case NodeKind.Crate: {
                if (patch.payload.kind != NodeKind.Module) {
                    throw Error(`${patch.payload.kind} cant't be payload for ${NodeKind.Crate} target`);
                }
                node.modules.push(patch.payload);
            }
                break;
            case NodeKind.Module: {
                if (patch.payload.kind == NodeKind.Module) {
                    if (node.modules == undefined) {
                        node.modules = [];
                    }
                    node.modules.push(patch.payload);
                } else if (patch.payload.kind == NodeKind.Function) {
                    if (node.modules == undefined) {
                        node.modules = [];
                    }
                    node.targets!.push(patch.payload);
                } else {
                    throw Error(`${patch.payload.kind} cant't be payload for ${NodeKind.Module} target`);
                }
            }
                break;
            case NodeKind.Function: {
                throw Error("Function can't be a target for Create's patch");
            }
        }
    }

    handleDelete(node: Node, parentField: Array<Node>) {
        const index = parentField.indexOf(node);
        parentField.splice(index, 1);
    }

    handleUpdate(node: Node, patch: Update) {
        switch (node.kind) {
            case NodeKind.Package: {
                node.location = patch.payload.location!;
            }
                break;
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

    public applyUpdate(update: DeltaUpdate) {
        for (const patch of update) {
            bfs(this.tree, (parentField, node) => {
                if (node.id == patch.targetId) {
                    switch (patch.kind) {
                        case PatchKind.Create: {
                            this.handleCreate(node, patch);
                        }
                            break;
                        case PatchKind.Delete: {
                            this.handleDelete(node, parentField);
                        }
                            break;
                        case PatchKind.Update: {
                            this.handleUpdate(node, patch);
                        }
                            break;
                    }
                }
            });
        }
        // this.treeChangeEmitter.fire(/* TODO */);
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
     * Creates run a set of tests.
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
     * Interrupts current run
     */
    async cancel() {
        this.client.sendRequest(ra.cancelTests);
    }
}

export class TestExplorerProvider {
    private controller: vscode.TestController;
    private treeDataProvider: vscode.TreeDataProvider<Node>;
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
        token.onCancellationRequested(() => this.testExecutor.cancel());

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
    constructor(treeProvider: vscode.TreeDataProvider<Node>, ctx: Ctx) {
        this.testExecutor = new TestRunControler(ctx);
        this.controller = vscode.tests.createTestController("rust-analyzer", "rust");
        this.treeDataProvider = treeProvider;
        this.treeDataProvider.onDidChangeTreeData!(this.updateBranch);

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

        this.updateBranch();
    }
}






