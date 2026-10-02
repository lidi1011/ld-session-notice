import { defineStore } from "pinia";
import { api } from "../api";
import type { DashboardSummary } from "../types";

export const useDashboardStore = defineStore("dashboard", {
  state: () => ({
    summary: null as DashboardSummary | null,
    loading: false,
    error: "",
  }),
  actions: {
    async load() {
      this.loading = true;
      this.error = "";
      try {
        this.summary = await api.dashboard();
      } catch (error) {
        this.error = String(error);
      } finally {
        this.loading = false;
      }
    },
  },
});
