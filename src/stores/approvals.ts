import { defineStore } from "pinia";
import { api } from "../api";
import type { PendingApproval } from "../types";

export const useApprovalStore = defineStore("approvals", {
  state: () => ({
    items: [] as PendingApproval[],
  }),
  actions: {
    async load() {
      this.items = await api.approvals();
    },
    async resolve(id: string, decision: "approved" | "rejected") {
      await api.resolveApproval(id, decision);
      await this.load();
    },
  },
});
