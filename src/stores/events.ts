import { defineStore } from "pinia";
import { api } from "../api";
import type { EventFilter, NoticeEvent } from "../types";

export const useEventsStore = defineStore("events", {
  state: () => ({
    items: [] as NoticeEvent[],
    filter: { search: "", level: "", project: "" } as EventFilter,
    page: 1,
    pageSize: 100,
    loading: false,
  }),
  actions: {
    async load() {
      this.loading = true;
      try {
        this.items = await api.events(this.filter, {
          page: this.page,
          pageSize: this.pageSize,
        });
      } finally {
        this.loading = false;
      }
    },
    async clear() {
      await api.clearEvents();
      await this.load();
    },
  },
});
