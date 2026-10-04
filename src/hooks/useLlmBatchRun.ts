import { useTranslation } from 'react-i18next';
import { useBatchTask } from './useBatchTask';
import { useBatchRunStats } from './useBatchRunStats';
import { useLlmApiConfig } from './useLlmApiConfig';
import { toIntervalMs } from '../utils/taggerOptions';

export interface LlmBatchStart<T> {
  taskName: string;
  /** 开始日志的 i18n 键，参数 model、api、threads、interval */
  startLogKey: string;
  /** 请求间隔（秒），负数表示无间隔 */
  intervalSec: number;
  concurrency: number;
  /** 收到换算好的 request_interval_ms，调用命令 */
  exec: (requestIntervalMs: number) => Promise<T>;
}

/**
 * 逐图请求 LLM 的批处理页（VLM 打标、标签细化、标签排序）共用：API 配置、成功/失败计数、任务与日志。
 * start 负责重置计数、写开始日志、运行，结束后汇总失败与异常文件。
 */
export function useLlmBatchRun({ event, taskId }: { event: string; taskId: string }) {
  const { t } = useTranslation();
  const api = useLlmApiConfig();
  const stats = useBatchRunStats();
  const task = useBatchTask({ event, taskId, onEvent: stats.onEvent });

  const start = async <T>({ taskName, startLogKey, intervalSec, concurrency, exec }: LlmBatchStart<T>) => {
    if (task.processing || !api.ready) return;
    stats.reset();
    const intervalMs = toIntervalMs(intervalSec);
    await task.run({
      taskName,
      startLog: t(startLogKey, {
        model: api.modelName,
        api: api.endpoint,
        threads: concurrency,
        interval: intervalMs < 0 ? t('llmApi.noInterval') : `${intervalMs / 1000}s`,
      }),
      exec: () => exec(intervalMs),
    });
    stats.summarize(task.logger);
  };

  const { onClearLogs } = task.progressLogProps;
  return {
    api,
    /** 命令参数里的 api_endpoint、api_key、model_name */
    apiOptions: { api_endpoint: api.endpoint, api_key: api.apiKey, model_name: api.modelName },
    processing: task.processing,
    buttonProps: task.buttonProps,
    progressLogProps: {
      ...task.progressLogProps,
      headerExtra: stats.headerExtra,
      onClearLogs: () => { onClearLogs(); stats.reset(); },
    },
    start,
  };
}
