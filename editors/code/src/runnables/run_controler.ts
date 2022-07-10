import { Ctx } from '../ctx';
import * as ra from '../lsp_ext';
import * as vscode from 'vscode';
import { RunStatusUpdate } from '../lsp_ext';
import { LanguageClient } from 'vscode-languageclient/node';

/**
 * Provides an API for creation and control tests run, and receiving notification of its
 * state.
 */
export class TestRunControler {
    private readonly client;
    private readonly emitter;

    constructor(client: LanguageClient) {
        this.client = client;
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