<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api } from "./api";
import { normalizeError, type AuthCred, type Favorite, type SvnVersion, type UiError } from "./types";
import RemoteView from "./components/RemoteView.vue";
import WcView from "./components/WcView.vue";
import TaskBar from "./components/TaskBar.vue";

const mode = ref<"remote" | "wc">("remote");
const wcPath = ref("");

function onCheckedOut(p: string): void {
  wcPath.value = p;
  mode.value = "wc";
}
const svn = ref<SvnVersion | null>(null);
const err = ref<UiError | null>(null);
const showDetail = ref(false);
const showSettings = ref(false);
const svnPath = ref("");
const settingsBusy = ref(false);
const settingsMsg = ref("");
// 认证缓存
const authCreds = ref<AuthCred[]>([]);
const authBusy = ref(false);
const authMsg = ref("");
// 收藏管理
const favs = ref<Favorite[]>([]);
const favBusy = ref(false);
const favMsg = ref("");

onMounted(async () => {
  await refreshSvn();
});

async function refreshSvn() {
  try {
    svn.value = await api.svnVersion();
  } catch (e) {
    err.value = normalizeError(e);
  }
}

async function saveSvnPath() {
  if (!svnPath.value.trim()) return;
  settingsBusy.value = true;
  settingsMsg.value = "";
  try {
    const v = await api.setSvnExecutable(svnPath.value.trim());
    svn.value = v;
    settingsMsg.value = `已生效：svn ${v.version}`;
  } catch (e) {
    settingsMsg.value = normalizeError(e).summary;
  } finally {
    settingsBusy.value = false;
  }
}

async function loadAuth(): Promise<void> {
  authBusy.value = true;
  authMsg.value = "";
  try {
    authCreds.value = await api.authList();
  } catch (e) {
    authMsg.value = normalizeError(e).summary;
  } finally {
    authBusy.value = false;
  }
}

async function removeAuth(cred: AuthCred): Promise<void> {
  authBusy.value = true;
  authMsg.value = "";
  try {
    const r = await api.authRemove([cred.realm]);
    authMsg.value = r.summary;
    await loadAuth();
  } catch (e) {
    authMsg.value = normalizeError(e).summary;
    authBusy.value = false;
  }
}

async function removeAllAuth(): Promise<void> {
  authBusy.value = true;
  authMsg.value = "";
  try {
    const r = await api.authRemove(["*"]);
    authMsg.value = r.summary;
    await loadAuth();
  } catch (e) {
    authMsg.value = normalizeError(e).summary;
    authBusy.value = false;
  }
}

function openSettings(): void {
  showSettings.value = true;
  void loadAuth();
  void loadFavs();
}

async function loadFavs(): Promise<void> {
  favBusy.value = true;
  favMsg.value = "";
  try {
    favs.value = await api.favList();
  } catch (e) {
    favMsg.value = normalizeError(e).summary;
  } finally {
    favBusy.value = false;
  }
}

async function removeFav(f: Favorite): Promise<void> {
  favBusy.value = true;
  favMsg.value = "";
  try {
    await api.favRemove(f.url);
    await loadFavs();
  } catch (e) {
    favMsg.value = normalizeError(e).summary;
    favBusy.value = false;
  }
}

async function clearFavs(): Promise<void> {
  favBusy.value = true;
  favMsg.value = "";
  try {
    await api.favClear();
    await loadFavs();
  } catch (e) {
    favMsg.value = normalizeError(e).summary;
    favBusy.value = false;
  }
}
</script>

<template>
  <main class="shell">
    <header class="topbar">
      <h1 class="title">SVN 图形化管理工具</h1>
      <nav class="tabs">
        <button :class="{ on: mode === 'remote' }" @click="mode = 'remote'">远程仓库</button>
        <button :class="{ on: mode === 'wc' }" @click="mode = 'wc'">工作副本</button>
      </nav>
      <span class="svnver" :title="svn?.bin ?? ''">svn {{ svn?.version ?? "…" }}</span>
      <button class="gear" @click="openSettings" title="设置">⚙</button>
    </header>

    <!-- 设置弹窗：svn 二进制路径（借鉴 OrcaSVN 的可配置 svn 可执行文件） -->
    <div v-if="showSettings" class="mask" @click.self="showSettings = false">
      <div class="dialog">
        <h3>设置</h3>
        <label class="row">
          <span>svn 可执行文件路径</span>
          <input
            v-model="svnPath"
            placeholder="/opt/homebrew/bin/svn"
            @keyup.enter="saveSvnPath"
          />
        </label>
        <p class="hint">当前：{{ svn?.bin ?? "未知" }}（svn {{ svn?.version ?? "…" }}）</p>
        <p class="msg">{{ settingsMsg }}</p>
        <div class="actions">
          <button :disabled="settingsBusy" @click="saveSvnPath">
            {{ settingsBusy ? "验证中…" : "保存并验证" }}
          </button>
          <button @click="showSettings = false">关闭</button>
        </div>

        <hr class="sep" />
        <div class="authrow">
          <b>认证缓存（svn auth）</b>
          <button class="small" :disabled="authBusy" @click="loadAuth">刷新</button>
          <button
            class="small danger"
            :disabled="authBusy || authCreds.length === 0"
            @click="removeAllAuth"
          >
            全部清除
          </button>
        </div>
        <p class="hint">已保存的服务器凭据 / 证书信任；清除后下次连接需重新输入。</p>
        <div v-if="authBusy" class="hint">加载中…</div>
        <ul v-else-if="authCreds.length > 0" class="authlist">
          <li v-for="c in authCreds" :key="c.kind + c.realm + c.username">
            <div class="authmain">
              <span class="authtag">{{ c.kind }}</span>
              <span class="authrealm" :title="c.raw">{{ c.realm }}</span>
              <span v-if="c.username" class="authuser">{{ c.username }}</span>
              <span v-if="c.fingerprint" class="authfp" :title="c.subject">
                {{ c.fingerprint.slice(0, 16) }}…
              </span>
            </div>
            <button class="small" :disabled="authBusy" @click="removeAuth(c)">清除</button>
          </li>
        </ul>
        <p v-else class="hint">无已保存的凭据</p>
        <p class="msg">{{ authMsg }}</p>

        <hr class="sep" />
        <div class="authrow">
          <b>收藏夹</b>
          <button class="small" :disabled="favBusy" @click="loadFavs">刷新</button>
          <button
            class="small danger"
            :disabled="favBusy || favs.length === 0"
            @click="clearFavs"
          >
            全部清除
          </button>
        </div>
        <p class="hint">远程仓库收藏（在远程页可一键切换）。</p>
        <ul v-if="favs.length > 0" class="authlist">
          <li v-for="f in favs" :key="f.url">
            <div class="authmain">
              <span class="authtag">★</span>
              <span class="authrealm" :title="f.url">{{ f.name }}</span>
            </div>
            <button class="small" :disabled="favBusy" @click="removeFav(f)">删除</button>
          </li>
        </ul>
        <p v-else class="hint">暂无收藏</p>
        <p class="msg">{{ favMsg }}</p>
      </div>
    </div>

    <RemoteView v-if="mode === 'remote'" class="content" @checked-out="onCheckedOut" />
    <WcView v-else class="content" :initial-path="wcPath" />

    <footer v-if="err" class="errbar">
      <div class="errline">
        <span class="errcat">{{ err.category }}</span>
        <b>{{ err.summary }}</b>
        <span class="errhint">{{ err.hint }}</span>
        <button @click="err = null">关闭</button>
        <button v-if="err.detail" @click="showDetail = !showDetail">
          {{ showDetail ? "收起" : "详情" }}
        </button>
      </div>
      <pre v-if="showDetail && err.detail" class="errdetail">{{ err.detail }}</pre>
    </footer>

    <TaskBar />
  </main>
</template>

<style scoped>
.shell {
  height: 100%;
  display: flex;
  flex-direction: column;
}
.topbar {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 8px 16px;
  background: #24292f;
  color: #fff;
}
.title {
  font-size: 15px;
  margin: 0;
  white-space: nowrap;
}
.tabs {
  display: flex;
  gap: 4px;
  flex: 1;
}
.tabs button {
  background: transparent;
  color: #c9d1d9;
  border: none;
  padding: 6px 14px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
}
.tabs button.on {
  background: #39424e;
  color: #fff;
}
.svnver {
  font-size: 12px;
  color: #8b949e;
  white-space: nowrap;
}
.gear {
  background: transparent;
  color: #c9d1d9;
  border: 1px solid #39424e;
  border-radius: 6px;
  padding: 2px 8px;
  cursor: pointer;
  font-size: 14px;
}
.mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}
.dialog {
  background: #fff;
  color: #1f2328;
  border-radius: 8px;
  padding: 16px 20px;
  width: 420px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.3);
}
.dialog h3 {
  margin: 0 0 12px;
  font-size: 15px;
}
.dialog .row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
}
.dialog input {
  padding: 6px 8px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  font-size: 13px;
  font-family: monospace;
}
.dialog .hint {
  font-size: 12px;
  color: #57606a;
  margin: 10px 0 0;
}
.dialog .msg {
  font-size: 12px;
  min-height: 16px;
  margin: 6px 0 0;
  color: #0969da;
}
.dialog .actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 14px;
}
.dialog .actions button {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 5px 14px;
  cursor: pointer;
  font-size: 13px;
}
.dialog .actions button:first-child {
  background: #1f883d;
  border-color: #1f883d;
  color: #fff;
}
.dialog .sep {
  border: none;
  border-top: 1px solid #e5e8eb;
  margin: 14px 0 10px;
}
.dialog .authrow {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
}
.dialog .small {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 3px 10px;
  cursor: pointer;
  font-size: 12px;
}
.dialog .small.danger {
  color: #c0392b;
  border-color: #c0392b;
}
.dialog .authlist {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
  max-height: 220px;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.dialog .authlist li {
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid #e5e8eb;
  border-radius: 6px;
  padding: 6px 8px;
}
.dialog .authmain {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}
.dialog .authtag {
  font-size: 11px;
  color: #fff;
  background: #57606a;
  border-radius: 4px;
  padding: 1px 6px;
  white-space: nowrap;
}
.dialog .authrealm {
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}
.dialog .authuser {
  font-size: 12px;
  color: #0969da;
  white-space: nowrap;
}
.dialog .authfp {
  font-size: 11px;
  color: #888;
  font-family: monospace;
  white-space: nowrap;
}
.content {
  flex: 1;
  min-height: 0;
  display: flex;
}
.errbar {
  border-top: 2px solid #cf222e;
  background: #fff5f5;
  padding: 6px 12px;
  font-size: 13px;
}
.errline {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.errcat {
  background: #cf222e;
  color: #fff;
  border-radius: 4px;
  padding: 1px 8px;
  font-size: 11px;
}
.errhint {
  color: #57606a;
  flex: 1;
}
.errbar button {
  font-size: 12px;
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 4px;
  padding: 2px 8px;
  cursor: pointer;
}
.errdetail {
  margin: 6px 0 2px;
  white-space: pre-wrap;
  max-height: 160px;
  overflow: auto;
  background: #f6f8fa;
  border: 1px solid #d0d7de;
  border-radius: 4px;
  padding: 8px;
  font-size: 12px;
}
</style>
