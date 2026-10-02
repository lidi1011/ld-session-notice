import { defineStore } from "pinia";
import { api } from "../api";
import type { ChannelConfig } from "../types";

export const useChannelStore = defineStore("channel", {
  state: () => ({
    config: null as ChannelConfig | null,
    webhookUrl: "",
    signSecret: "",
    lastMessage: "",
    saving: false,
    testing: false,
    error: "",
  }),
  actions: {
    async load() {
      this.error = "";
      try {
        this.config = await api.channelConfig();
      } catch (error) {
        this.error = String(error);
      }
    },
    async save() {
      this.saving = true;
      this.error = "";
      this.lastMessage = "";
      try {
        this.config = await api.saveFeishuConfig(this.webhookUrl, this.signSecret || undefined);
        this.webhookUrl = "";
        this.signSecret = "";
        this.lastMessage = "飞书配置已保存";
      } catch (error) {
        this.error = String(error);
      } finally {
        this.saving = false;
      }
    },
    async test() {
      this.testing = true;
      this.error = "";
      this.lastMessage = "";
      try {
        this.lastMessage = await api.testFeishu();
        await this.load();
      } catch (error) {
        this.error = String(error);
      } finally {
        this.testing = false;
      }
    },
    async setEnabled(enabled: boolean) {
      this.error = "";
      this.lastMessage = "";
      try {
        this.config = await api.setFeishuEnabled(enabled);
        this.lastMessage = enabled ? "飞书通知已开启" : "飞书通知已关闭";
      } catch (error) {
        this.error = String(error);
      }
    },
  },
});
