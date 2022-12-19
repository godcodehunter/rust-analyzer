export type TestId = string;
export type TestLocation = string;
export type TextRange = [[number, number], [number, number]];

/// Defines test tree parts of which 

export type Node = Session | Crate | Module | Function;

export enum NodeKind {
    Session = "Session",
    Crate = "Crate",
    Module = "Module",
    Function = "Function",
}

export interface Session {
    tag: NodeKind.Session;
    crates: Crate[];
}

export interface Crate {
    tag: NodeKind.Crate;
    id: TestId;
    name: string;
    location: TestLocation;
    modules: Module[];
}

export interface Module {
    tag: NodeKind.Module;
    id: TestId;
    name: string;
    location: TestLocation;
    modules?: Module[];
    targets?: Function[];
}

export enum TestKind {
    Test,
    Bench,
    Bin,
}

export interface Function {
    tag: NodeKind.Function;
    id: TestId;
    name: string;
    location: TestLocation;
    range: TextRange;
    testKind: TestKind;
}

/// The view synchronized with server data by `DeltaUpdate`'s. The update is an array   
/// of elementary actions called a `Patch`. After applying an update to the tree 
/// it will become synchronized.
///
/// All groups are transitive among themselves, in addition the Update and Delete 
/// patches are transitive in a group, but Append is not transitive in a group 
/// and must be applied in order

export interface DeltaUpdate {
    id: TestId,
    delete: Delete[],
    update: Update[],
    append: Append[]
}

interface Delete {
    targetId: TestId;
}

export interface Update {
    targetId: TestId;
    payload: {
        name?: string;
        location?: string;
        testKind?: TestKind;
    };
}

type AppendItem = Crate | Module | Function;

interface Append {
    targetId: TestId;
    item: AppendItem;
}
