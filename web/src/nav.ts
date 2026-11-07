import type { MainView, UnlockView } from "./api";

/** Page switching; main.ts installs the implementations so views do not import each other. */
export const nav = {
  unlock: (_view: UnlockView): void => {},
  main: (_view: MainView): void => {},
};
