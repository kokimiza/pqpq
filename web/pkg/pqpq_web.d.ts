/* tslint:disable */
/* eslint-disable */

export class Client {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Flat `[labelIndex, x, y, direction, isMe]` per car, labelIndex -1 if none.
     */
    cars(now_ms: number): Float64Array;
    /**
     * Transport ended; shown unless the player left on purpose.
     */
    closed(reason: string): void;
    join(username: string, room_id: string): void;
    leave(): void;
    neutral(): void;
    constructor();
    on_datagram(bytes: Uint8Array, now_ms: number): void;
    on_stream(bytes: Uint8Array, now_ms: number): void;
    ready(): void;
    set_keys(up: boolean, down: boolean, left: boolean, right: boolean): void;
    take_datagram(): Uint8Array | undefined;
    /**
     * Bytes for the control stream (possibly empty).
     */
    take_stream(): Uint8Array;
    update(now_ms: number): void;
    /**
     * Everything the page shows, as a plain object.
     */
    view(now_ms: number): any;
}

/**
 * Track geometry for drawing, straight from the shared simulation.
 */
export function course_geometry(): any;

export function fixed_dt_seconds(): number;

export function physics_version(): number;

export function protocol_version(): number;

/**
 * Known Join bytes, for web/check.mjs.
 */
export function reference_join_frame(): Uint8Array;

/**
 * Same input sequence as the native test, for web/check.mjs.
 */
export function reference_state(): Float64Array;

export function tick_hz(): number;

/**
 * Form check with the same rules as the server; `undefined` when valid.
 */
export function validate(username: string, room_id: string): string | undefined;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_client_free: (a: number, b: number) => void;
    readonly client_cars: (a: number, b: number) => [number, number];
    readonly client_closed: (a: number, b: number, c: number) => void;
    readonly client_join: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly client_leave: (a: number) => void;
    readonly client_neutral: (a: number) => void;
    readonly client_new: () => number;
    readonly client_on_datagram: (a: number, b: number, c: number, d: number) => void;
    readonly client_on_stream: (a: number, b: number, c: number, d: number) => [number, number];
    readonly client_ready: (a: number) => void;
    readonly client_set_keys: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly client_take_datagram: (a: number) => [number, number];
    readonly client_take_stream: (a: number) => [number, number];
    readonly client_update: (a: number, b: number) => void;
    readonly client_view: (a: number, b: number) => any;
    readonly course_geometry: () => any;
    readonly fixed_dt_seconds: () => number;
    readonly physics_version: () => number;
    readonly protocol_version: () => number;
    readonly reference_join_frame: () => [number, number];
    readonly reference_state: () => [number, number];
    readonly tick_hz: () => number;
    readonly validate: (a: number, b: number, c: number, d: number) => [number, number];
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
