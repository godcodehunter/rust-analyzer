export type Node = Session | Crate | Module | Function;

export enum NodeKind {
    Session = "Session",
    Crate = "Crate",
    Module = "Module",
    Function = "Function",
}

export interface Session {
    kind: NodeKind.Session;
    crates: Crate[];
}

export interface Crate {
    kind: NodeKind.Crate;
    id: string;
    name: string;
    modules: Module[];
    location: string;
}

export interface Module {
    kind: NodeKind.Module;
    id: string;
    name: string;
    modules?: Module[];
    targets?: Function[];
    location: string;
}

export enum TestKind {
    Test,
    Bench,
    Bin,
}

export interface Function {
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

export interface DeltaUpdate {
    id: number,
    delete: Delete[],
    update: Update[],
    append: Append[]
}

interface Delete {
    targetId: number;
}

export interface Update {
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
