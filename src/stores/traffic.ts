import { defineStore } from "pinia";
import { api } from "../api";
import type { TrafficWidgetManualState, TrafficWidgetStatus } from "../types";

export const useTrafficStore = defineStore("traffic", {
  state: () => ({
    status: null as TrafficWidgetStatus | null,
    loading: false,
    error: "",
  }),
  actions: {
    async load() {
      this.loading = true;
      this.error = "";
      try {
        this.status = await api.trafficStatus();
      } catch (error) {
        this.error = String(error);
      } finally {
        this.loading = false;
      }
    },
    async setEnabled(enabled: boolean) {
      this.error = "";
      try {
        this.status = await api.setTrafficWidgetEnabled(enabled);
      } catch (error) {
        this.error = String(error);
      }
    },
    async setAlwaysOnTop(alwaysOnTop: boolean) {
      this.error = "";
      try {
        this.status = await api.setTrafficWidgetAlwaysOnTop(alwaysOnTop);
      } catch (error) {
        this.error = String(error);
      }
    },
    async setManualOverride(manualState?: TrafficWidgetManualState) {
      this.loading = true;
      this.error = "";
      try {
        this.status = await api.setTrafficWidgetManualOverride(manualState);
      } catch (error) {
        this.error = String(error);
        throw error;
      } finally {
        this.loading = false;
      }
    },
    async refreshRuntimeStatus() {
      this.loading = true;
      this.error = "";
      try {
        this.status = await api.refreshRuntimeStatus();
      } catch (error) {
        this.error = String(error);
        throw error;
      } finally {
        this.loading = false;
      }
    },
  },
});
