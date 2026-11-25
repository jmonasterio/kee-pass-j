import { api, errorText, type UpdateInfo } from "./api";
import { toast } from "./toast";

/** Offers a found update in a toast that stays until "Update now" is pressed. */
export function offerUpdate(info: UpdateInfo): void {
  toast(`Our Logins ${info.version} is available (you have ${info.current}).`, { label: "Update now", run: () => void install() }, true);
}

async function install(): Promise<void> {
  toast("Downloading the update. Our Logins restarts when it is installed.", undefined, true);
  try {
    // Returns only on failure: on success the app restarts.
    await api.updateInstall();
  } catch (err) {
    toast(errorText(err));
  }
}

/** Silent at startup (no network, no update: nothing shown); `manual` also reports those. */
export async function checkForUpdates(manual: boolean): Promise<void> {
  try {
    const info = await api.updateCheck();
    if (info) offerUpdate(info);
    else if (manual) toast("Our Logins is up to date.");
  } catch (err) {
    if (manual) toast(errorText(err));
  }
}
