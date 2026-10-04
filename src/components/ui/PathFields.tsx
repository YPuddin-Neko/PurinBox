import { useId, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import RecursiveScanToggle from '../RecursiveScanToggle';
import PathInput from './PathInput';
import './ui.css';

/** output 为 undefined 时不渲染输出行（如筛选页的删除模式：output={needsOutput ? outputPath : undefined}） */
type OutputFieldProps =
  | { output: string | undefined; onOutput: (path: string) => void; outputLabel?: string }
  | { output?: undefined; onOutput?: undefined; outputLabel?: undefined };

/** 不传 recursive 时不显示递归开关 */
type RecursiveFieldProps =
  | { recursive: boolean; onRecursive: (recursive: boolean) => void }
  | { recursive?: undefined; onRecursive?: undefined };

/** embedded：只渲染两行表单，由外层面板排版；否则自带 tool-panel 外壳 */
type LayoutProps =
  | { embedded: true; title?: undefined; headerExtra?: undefined; children?: undefined }
  | {
    embedded?: false;
    /** 面板标题，默认 pages.pathSettings */
    title?: string;
    /** 面板标题栏右侧内容（如批大小、CPU/GPU 切换） */
    headerExtra?: ReactNode;
    /** 面板内、路径行之后的内容 */
    children?: ReactNode;
  };

export type PathFieldsProps = OutputFieldProps & RecursiveFieldProps & LayoutProps & {
  input: string;
  onInput: (path: string) => void;
  /** 输入可以是单张图片（后端支持单文件输入时打开） */
  allowFile?: boolean;
};

export default function PathFields(props: PathFieldsProps) {
  const { t } = useTranslation();
  const inputId = useId();
  const outputId = useId();
  const { input, onInput, allowFile = false } = props;

  const fields = (
    <>
      <div className="form-group">
        <div className="form-label-row">
          <label className="form-label" htmlFor={inputId}>{t('pages.inputPathShort')}</label>
          {props.recursive !== undefined && (
            <RecursiveScanToggle checked={props.recursive} onChange={props.onRecursive} />
          )}
        </div>
        <PathInput id={inputId} value={input} onChange={onInput} pick={allowFile ? 'folderOrImage' : 'folder'} />
      </div>
      {props.output !== undefined && (
        <div className="form-group">
          <label className="form-label" htmlFor={outputId}>{props.outputLabel ?? t('pages.outputPath')}</label>
          <PathInput id={outputId} kind="output" value={props.output} onChange={props.onOutput} />
        </div>
      )}
    </>
  );

  if (props.embedded) return fields;

  return (
    <div className="tool-panel">
      <div className="tool-panel-header">
        <span className="tool-panel-title">{props.title ?? t('pages.pathSettings')}</span>
        {props.headerExtra}
      </div>
      <div className="ui-stack">
        {fields}
        {props.children}
      </div>
    </div>
  );
}
