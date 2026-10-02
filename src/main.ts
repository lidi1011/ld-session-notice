import { createApp } from "vue";
import { createPinia } from "pinia";
import {
  darkTheme,
  dateZhCN,
  zhCN,
  NButton,
  NCard,
  NConfigProvider,
  NDataTable,
  NDialogProvider,
  NForm,
  NFormItem,
  NInput,
  NLayout,
  NLayoutContent,
  NLayoutSider,
  NMenu,
  NMessageProvider,
  NModal,
  NSpace,
  NStatistic,
  NSwitch,
  NTag,
} from "naive-ui";
import App from "./App.vue";
import "./styles.css";

const app = createApp(App);

app.use(createPinia());
app.component("NButton", NButton);
app.component("NCard", NCard);
app.component("NConfigProvider", NConfigProvider);
app.component("NDataTable", NDataTable);
app.component("NDialogProvider", NDialogProvider);
app.component("NForm", NForm);
app.component("NFormItem", NFormItem);
app.component("NInput", NInput);
app.component("NLayout", NLayout);
app.component("NLayoutContent", NLayoutContent);
app.component("NLayoutSider", NLayoutSider);
app.component("NMenu", NMenu);
app.component("NMessageProvider", NMessageProvider);
app.component("NModal", NModal);
app.component("NSpace", NSpace);
app.component("NStatistic", NStatistic);
app.component("NSwitch", NSwitch);
app.component("NTag", NTag);

app.provide("naiveTheme", darkTheme);
app.provide("naiveLocale", zhCN);
app.provide("naiveDateLocale", dateZhCN);
app.mount("#app");
