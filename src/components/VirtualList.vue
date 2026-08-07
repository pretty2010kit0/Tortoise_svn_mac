<script setup lang="ts" generic="T">
// 轻量虚拟滚动列表：固定行高 + 可视窗口（±buffer），只渲染可见行。
// 用于 svn list / svn log 的全量结果（服务端无分页），数千行时保持滚动流畅。
import { computed, onMounted, ref, watch } from "vue";

const props = withDefaults(
  defineProps<{
    items: T[];
    rowHeight?: number;
    buffer?: number;
    minHeight?: string;
  }>(),
  { rowHeight: 30, buffer: 8, minHeight: "300px" },
);

const scroller = ref<HTMLDivElement | null>(null);
const scrollTop = ref(0);
const viewportH = ref(0);

const totalHeight = computed(() => props.items.length * props.rowHeight);

/** 可见窗口起点（含 buffer） */
const start = computed(() => {
  const max = Math.max(0, props.items.length - 1);
  return Math.min(
    Math.max(0, Math.floor(scrollTop.value / props.rowHeight) - props.buffer),
    max,
  );
});

/** 可见窗口终点（含 buffer） */
const end = computed(() => {
  const visible = Math.ceil(viewportH.value / props.rowHeight) + props.buffer;
  return Math.min(props.items.length, start.value + visible);
});

const visibleItems = computed(() => props.items.slice(start.value, end.value));

function onScroll(): void {
  const el = scroller.value;
  if (!el) return;
  scrollTop.value = el.scrollTop;
  viewportH.value = el.clientHeight;
}

// 数据源整体变化（切换目录/过滤日志）时回到顶部
watch(
  () => props.items,
  () => {
    if (scroller.value) scroller.value.scrollTop = 0;
    scrollTop.value = 0;
  },
);

onMounted(() => {
  const el = scroller.value;
  if (el) viewportH.value = el.clientHeight;
});
</script>

<template>
  <div
    ref="scroller"
    class="vlist"
    :style="{ minHeight }"
    @scroll="onScroll"
  >
    <div class="vlist-spacer" :style="{ height: `${totalHeight}px` }">
      <div
        class="vlist-window"
        :style="{ transform: `translateY(${start * rowHeight}px)` }"
      >
        <template v-for="(item, i) in visibleItems" :key="i">
          <slot name="row" :item="item" :index="start + i" />
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.vlist {
  overflow: auto;
  flex: 1;
  min-height: 0;
}
.vlist-spacer {
  position: relative;
  width: 100%;
}
.vlist-window {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
}
</style>
