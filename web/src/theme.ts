import { setTheme } from "@fluentui/web-components";
import { webDarkTheme, webLightTheme } from "@fluentui/tokens";
import type { PlatformInfo } from "./api";

const FONTS: Record<PlatformInfo["os"], { base?: string; mono?: string }> = {
  windows: {
    base: "'Segoe UI Variable Text', 'Segoe UI Variable', 'Segoe UI', sans-serif",
    mono: "Consolas, 'Cascadia Mono', 'Courier New', monospace",
  },
  macos: {
    base: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Helvetica Neue', sans-serif",
    mono: "Menlo, 'SF Mono', Monaco, monospace",
  },
  linux: {},
};

/** Applies the Fluent light/dark theme and keeps following the OS color scheme. */
export function initTheme(os: PlatformInfo["os"]): void {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const fonts = FONTS[os];
  const apply = () => {
    const dark = query.matches;
    const theme = { ...(dark ? webDarkTheme : webLightTheme) };
    if (fonts.base) theme.fontFamilyBase = fonts.base;
    if (fonts.mono) theme.fontFamilyMonospace = fonts.mono;
    setTheme(theme);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
  };
  apply();
  query.addEventListener("change", apply);
}
