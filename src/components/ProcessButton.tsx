import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Play, Loader2, X, AlertTriangle } from 'lucide-react';

/**
 * 统一的处理/取消按钮组件
 *
 * 行为:
 * - 空闲时：显示 startText（如"开始处理"），点击触发 onStart
 * - 处理中未 hover：显示 processingText + 旋转图标
 * - 处理中 hover：显示"取消"，红色
 * - 第一次点击取消：调用 onCancel，按钮变为"再次点击强制结束"
 * - 第二次点击取消：调用 onForceCancel
 */

interface ProcessButtonProps {
  processing: boolean;
  disabled?: boolean;
  onStart: () => void;
  /** 优雅取消 */
  cancelCommand: string;
  /** 强制取消（可选，默认同 cancelCommand） */
  forceCancelCommand?: string;
  startText?: string;
  startIcon?: React.ReactNode;
  processingText?: string;
  style?: React.CSSProperties;
  /** 取消时自动追加日志 */
  onCancelLog?: (msg: string) => void;
}

export default function ProcessButton({
  processing,
  disabled,
  onStart,
  cancelCommand,
  forceCancelCommand,
  startText = '开始处理',
  startIcon,
  processingText = '处理中...',
  style,
  onCancelLog,
}: ProcessButtonProps) {
  const [hovered, setHovered] = useState(false);
  const [cancelStage, setCancelStage] = useState(0); // 0=none, 1=graceful, 2=force

  const handleClick = useCallback(async () => {
    if (!processing) {
      setCancelStage(0);
      onStart();
      return;
    }

    if (cancelStage === 0) {
      // 第一次取消 → 优雅取消
      setCancelStage(1);
      onCancelLog?.('取消任务已提交，等待当前作业完毕后停止...');
      try {
        await invoke(cancelCommand);
      } catch (e) {
        console.error('cancel failed:', e);
      }
    } else {
      // 第二次取消 → 强制取消
      setCancelStage(2);
      onCancelLog?.('强制结束任务');
      try {
        await invoke(forceCancelCommand || cancelCommand);
      } catch (e) {
        console.error('force cancel failed:', e);
      }
    }
  }, [processing, cancelStage, cancelCommand, forceCancelCommand, onCancelLog, onStart]);

  // 处理完成后重置 cancelStage
  if (!processing && cancelStage > 0) {
    setTimeout(() => setCancelStage(0), 0);
  }

  // 决定显示内容
  const renderContent = () => {
    if (!processing) {
      return <>{startIcon || <Play style={{ width: 18, height: 18 }} />} {startText}</>;
    }

    if (cancelStage >= 1) {
      // 已请求取消
      return (
        <>
          <AlertTriangle style={{ width: 18, height: 18 }} />
          {cancelStage === 2 ? '正在强制结束...' : '再次点击强制结束'}
        </>
      );
    }

    if (hovered) {
      return <><X style={{ width: 18, height: 18 }} /> 取消</>;
    }

    return <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> {processingText}</>;
  };

  // 决定样式
  const isCancel = processing && (hovered || cancelStage >= 1);
  const btnClass = `btn ${isCancel ? '' : 'btn-primary'} btn-lg`;
  const btnStyle: React.CSSProperties = {
    width: '100%',
    height: 48,
    transition: 'all 0.15s ease',
    ...style,
    ...(isCancel ? {
      background: cancelStage >= 1
        ? 'rgba(248, 113, 113, 0.15)'
        : 'rgba(248, 113, 113, 0.1)',
      color: '#f87171',
      border: '1px solid rgba(248, 113, 113, 0.3)',
    } : {}),
  };

  return (
    <button
      className={btnClass}
      style={btnStyle}
      onClick={handleClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      disabled={!processing && disabled}
    >
      {renderContent()}
    </button>
  );
}
