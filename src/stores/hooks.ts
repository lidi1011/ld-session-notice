import { defineStore } from "pinia";
import { api } from "../api";
import type { HookPreview, HookStatus } from "../types";

export const useHookStore = defineStore("hooks", {
  state: () => ({
    status: null as HookStatus | null,
    preview: null as HookPreview | null,
  }),
  actions: {
    async load() {
      this.status = await api.hookStatus();
    },
    async previewInstall() {
      this.preview = await api.previewHookInstall();
    },
    async install() {
      this.status = await api.installHooks();
      await this.previewInstall();
    },
    async uninstall() {
      this.status = await api.uninstallHooks();
      await this.previewInstall();
    },
  },
});
