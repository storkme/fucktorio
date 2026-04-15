export interface DebugState {
  master: boolean;
  stepThrough: boolean;
  validation: boolean;
  satZones: boolean;
  soloRegions: boolean;
}

type Subscriber = (state: DebugState) => void;

let state: DebugState = {
  master: false,
  stepThrough: true,
  validation: false,
  satZones: false,
  soloRegions: false,
};

const subs: Subscriber[] = [];

export function create(): void {
  const fromParam = new URLSearchParams(window.location.search).get("debug") === "1";
  const fromStorage = localStorage.getItem("fk-debug") === "1";
  state = { ...state, master: fromParam || fromStorage };
}

export function get(): DebugState {
  return state;
}

export function set(patch: Partial<DebugState>): void {
  state = { ...state, ...patch };
  if ("master" in patch) {
    localStorage.setItem("fk-debug", patch.master ? "1" : "0");
  }
  for (const cb of subs) cb(state);
}

export function subscribe(cb: Subscriber): () => void {
  subs.push(cb);
  return () => {
    const i = subs.indexOf(cb);
    if (i >= 0) subs.splice(i, 1);
  };
}
