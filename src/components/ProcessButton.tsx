import { useState, useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Play, Loader2, X, AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';

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
  /** 取消时自动追加日志 */
  onCancelLog?: (msg: string) => void;
}

/**
 * 空闲时点击调用 onStart；处理中第一次点击 invoke(cancelCommand)，
 * 第二次 invoke(forceCancelCommand || cancelCommand)。
 */
export default function ProcessButton({
  processing,
  disabled,
  onStart,
  cancelCommand,
  forceCancelCommand,
  startText,
  startIcon,
  processingText,
  onCancelLog,
}: ProcessButtonProps) {
  const { t } = useTranslation();
  const resolvedStartText = startText || t('processButton.start');
  const resolvedProcessingText = processingText || t('processButton.processing');
  const [hovered, setHovered] = useState(false);
  const [cancelStage, setCancelStage] = useState(0); // 0 未取消，1 已请求取消，2 已强制结束
  const processingSinceRef = useRef(0);
  useEffect(() => {
    if (processing) processingSinceRef.current = Date.now();
    else setCancelStage(0);
  }, [processing]);

  const handleClick = useCallback(async () => {
    if (!processing) {
      onStart();
      return;
    }

    // 双击"开始"防抖：processing 刚翻转时按钮已原地变成取消，第二击不应立刻取消刚启动的任务
    if (Date.now() - processingSinceRef.current < 400) return;

    if (cancelStage === 0) {
      setCancelStage(1);
      onCancelLog?.(t('processButton.cancelSubmitted'));
      try {
        await invoke(cancelCommand);
      } catch (e) {
        console.error('cancel failed:', e);
      }
    } else {
      setCancelStage(2);
      onCancelLog?.(t('processButton.forceStop'));
      try {
        await invoke(forceCancelCommand || cancelCommand);
      } catch (e) {
        console.error('force cancel failed:', e);
      }
    }
  }, [processing, cancelStage, cancelCommand, forceCancelCommand, onCancelLog, onStart, t]);

  const renderContent = () => {
    if (!processing) {
      return <>{startIcon || <Play style={{ width: 18, height: 18 }} />} {resolvedStartText}</>;
    }

    if (cancelStage >= 1) {
      return (
        <>
          <AlertTriangle style={{ width: 18, height: 18 }} />
          {cancelStage === 2 ? t('processButton.forceEnding') : t('processButton.clickForce')}
        </>
      );
    }

    if (hovered) {
      return <><X style={{ width: 18, height: 18 }} /> {t('processButton.cancel')}</>;
    }

    return <><Loader2 style={{ width: 18, height: 18, animation: 'spin 1s linear infinite' }} /> {resolvedProcessingText}</>;
  };

  const isCancel = processing && (hovered || cancelStage >= 1);
  const btnClass = `btn ${isCancel ? '' : 'btn-primary'} btn-lg`;
  const btnStyle: React.CSSProperties = {
    width: '100%',
    height: 48,
    transition: 'all 0.15s ease',
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
