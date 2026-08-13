import {
  CheckCircleFilled,
  CloseCircleFilled,
  LoadingOutlined,
  MinusCircleFilled,
  PauseCircleFilled,
} from '@ant-design/icons';
import { SessionStatus } from '../types';

/**
 * 后端会话状态机 → 前端展示的唯一映射表（ZER-669）。
 *
 * 后端状态机为 starting / idle / running / suspended / error / exited，
 * 这里只做展示层映射（文案 / 颜色 / 图标），不改后端状态机。所有状态
 * 相关的展示配置都集中在这一个文件维护，筛选下拉、表格 valueEnum、
 * 状态标签都从这里的同一份映射派生。
 */
export interface SessionStatusDisplay {
  label: string;
  color: string;
  icon: React.ReactNode;
}

export const SESSION_STATUS_DISPLAY: Record<SessionStatus, SessionStatusDisplay> = {
  starting: { label: '启动中', color: 'processing', icon: <LoadingOutlined spin /> },
  idle: { label: '空闲', color: 'default', icon: <MinusCircleFilled /> },
  running: { label: '运行中', color: 'success', icon: <CheckCircleFilled /> },
  suspended: { label: '等待', color: 'warning', icon: <PauseCircleFilled /> },
  error: { label: '失败', color: 'error', icon: <CloseCircleFilled /> },
  exited: { label: '完成', color: 'default', icon: <MinusCircleFilled /> },
  unknown: { label: '未知', color: 'default', icon: <MinusCircleFilled /> },
};

export function sessionStatusDisplay(status: SessionStatus): SessionStatusDisplay {
  return SESSION_STATUS_DISPLAY[status] || SESSION_STATUS_DISPLAY.unknown;
}

/** 真实后端状态（不含 unknown）的筛选选项，供筛选表单使用。 */
export const SESSION_STATUS_FILTER_OPTIONS: Array<{
  label: string;
  value: SessionStatus;
}> = (
  ['starting', 'idle', 'running', 'suspended', 'error', 'exited'] as SessionStatus[]
).map((value) => ({ value, label: SESSION_STATUS_DISPLAY[value].label }));

/** 同一份映射的 ProTable valueEnum 形态。 */
export const SESSION_STATUS_VALUE_ENUM: Record<string, { text: string }> =
  Object.fromEntries(
    SESSION_STATUS_FILTER_OPTIONS.map((option) => [
      option.value,
      { text: option.label },
    ]),
  );
