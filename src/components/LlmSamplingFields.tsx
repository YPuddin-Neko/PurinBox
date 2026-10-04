import { useId, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Focus, ImageIcon, Layers, Thermometer, Timer } from 'lucide-react';
import NumberInput from './ui/NumberInput';
import CustomSelect from './CustomSelect';
import { IMAGE_DETAILS, isOneOf, type ImageDetail } from '../api/commandOptions';
import { IMAGE_DETAIL_OPTIONS } from '../utils/imageDetail';

/** LLM 请求参数的界面值（请求间隔按秒，负数表示无间隔） */
export interface LlmSampling {
  intervalSec: number;
  concurrency: number;
  temperature: number;
  topP: number;
  imageSize: number;
  imageDetail: ImageDetail;
}

/** 各页面在此基础上覆盖自己的默认值（如温度） */
export const LLM_SAMPLING_DEFAULTS: Readonly<LlmSampling> = {
  intervalSec: -1,
  concurrency: 1,
  temperature: 0.3,
  topP: 0,
  imageSize: 1024,
  imageDetail: '',
};

interface Props {
  value: LlmSampling;
  onChange: (value: LlmSampling) => void;
  /**
   * panel：表单面板里的两列行（请求间隔与并发、温度与 Top P，可选图片行）；
   * compact：辅助打标的窄栏（温度、Top P 各占一行，图片尺寸、并发、间隔一行，图像细节与 extra 一行）
   */
  layout?: 'panel' | 'compact';
  /** 显示图片发送尺寸与图像细节 */
  image?: boolean;
  /** panel 布局：温度与 Top P 一行放在最前，请求间隔与并发放在最后 */
  samplingFirst?: boolean;
  /** 图片行里的附加字段：panel 布局放在图片尺寸之后，compact 布局放在图像细节之后 */
  extra?: ReactNode;
  /** 并发上限，默认 32 */
  maxConcurrency?: number;
  /** 图片发送尺寸输入框清空时的占位文字 */
  imageSizePlaceholder?: string;
}

const ICON = { width: 13, height: 13, color: 'var(--color-text-tertiary)' } as const;
const SMALL_ICON = { width: 12, height: 12, color: 'var(--color-text-tertiary)' } as const;
const VALUE = { fontSize: 11, color: 'var(--color-accent-primary)', fontFamily: 'monospace' } as const;
const RANGE = { width: '100%', accentColor: 'var(--color-accent-primary)' } as const;
const PANEL_ROW = { display: 'flex', gap: 'var(--space-3)' } as const;
const PANEL_FIELD = { marginBottom: 0, flex: 1 } as const;
const COMPACT_ROW = { display: 'flex', gap: 'var(--space-2)' } as const;

/** LLM 请求的间隔、并发、温度、Top P，以及可选的图片发送尺寸与图像细节 */
export default function LlmSamplingFields({
  value, onChange, layout = 'panel', image = false, samplingFirst = false, extra, maxConcurrency = 32, imageSizePlaceholder,
}: Props) {
  const { t } = useTranslation();
  const id = useId();
  const set = <K extends keyof LlmSampling>(key: K) => (next: LlmSampling[K]) => onChange({ ...value, [key]: next });

  const interval = (
    <NumberInput id={`${id}-interval`} min={-1} max={120} step={1} fallback={-1} title={t('llmApi.intervalTip')}
      value={value.intervalSec} onChange={set('intervalSec')} />
  );
  const concurrency = (
    <NumberInput id={`${id}-concurrency`} integer min={1} max={maxConcurrency} step={1} fallback={1}
      title={layout === 'panel' ? t('llmApi.concurrencyTip') : undefined}
      value={value.concurrency} onChange={set('concurrency')} />
  );
  const imageSize = (
    <NumberInput id={`${id}-image-size`} integer min={256} max={4096} step={64} fallback={1024} placeholder={imageSizePlaceholder}
      value={value.imageSize} onChange={set('imageSize')} />
  );
  const imageDetail = (
    <CustomSelect value={value.imageDetail} options={IMAGE_DETAIL_OPTIONS(t)}
      onChange={v => { if (isOneOf(IMAGE_DETAILS, v)) set('imageDetail')(v); }} />
  );
  const range = (key: 'temperature' | 'topP', max: number) => (
    <input id={`${id}-${key}`} type="range" min="0" max={max} step="0.05" value={value[key]}
      onChange={e => set(key)(Number(e.target.value))} style={RANGE} />
  );
  const temperatureLabel = <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Thermometer style={ICON} /> {t('llmApi.temperature')}</span>;

  if (layout === 'compact') {
    return <>
      <div>
        <label className="form-label" htmlFor={`${id}-temperature`} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
          {temperatureLabel}
          <span style={VALUE}>{value.temperature}</span>
        </label>
        {range('temperature', 2)}
      </div>
      <div>
        <label className="form-label" htmlFor={`${id}-topP`} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
          <span>Top P</span>
          <span style={VALUE}>{value.topP}</span>
        </label>
        {range('topP', 1)}
      </div>
      <div style={COMPACT_ROW}>
        {image && (
          <div style={{ flex: 1 }}>
            <label className="form-label" htmlFor={`${id}-image-size`} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
              <ImageIcon style={SMALL_ICON} /> {t('llmApi.imageSize')}
            </label>
            {imageSize}
          </div>
        )}
        <div style={{ flex: 1 }}>
          <label className="form-label" htmlFor={`${id}-concurrency`}>{t('llmApi.concurrency')}</label>
          {concurrency}
        </div>
        <div style={{ flex: 1 }}>
          <label className="form-label" htmlFor={`${id}-interval`}>{t('llmApi.interval')}</label>
          {interval}
        </div>
      </div>
      {image && (
        <div style={COMPACT_ROW}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
              <Focus style={SMALL_ICON} /> {t('llmApi.imageDetail')}
            </label>
            {imageDetail}
          </div>
          {extra}
        </div>
      )}
    </>;
  }

  const requestRow = (
    <div style={PANEL_ROW}>
      <div className="form-group" style={PANEL_FIELD}>
        <label className="form-label" htmlFor={`${id}-interval`} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <Timer style={ICON} /> {t('llmApi.interval')}
        </label>
        {interval}
      </div>
      <div className="form-group" style={PANEL_FIELD}>
        <label className="form-label" htmlFor={`${id}-concurrency`} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <Layers style={ICON} /> {t('llmApi.concurrency')}
        </label>
        {concurrency}
      </div>
    </div>
  );
  const samplingRow = (
    <div style={PANEL_ROW}>
      <div className="form-group" style={PANEL_FIELD}>
        <label className="form-label" htmlFor={`${id}-temperature`} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          {temperatureLabel}
          <span style={VALUE}>{value.temperature}</span>
        </label>
        {range('temperature', 2)}
      </div>
      <div className="form-group" style={PANEL_FIELD}>
        <label className="form-label" htmlFor={`${id}-topP`} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span>Top P</span>
          <span style={VALUE}>{value.topP}</span>
        </label>
        {range('topP', 1)}
      </div>
    </div>
  );
  const imageRow = image && (
    <div style={PANEL_ROW}>
      <div className="form-group" style={PANEL_FIELD}>
        <label className="form-label" htmlFor={`${id}-image-size`} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <ImageIcon style={ICON} /> {t('llmApi.imageSize')}
        </label>
        {imageSize}
      </div>
      {extra}
      <div className="form-group" style={PANEL_FIELD}>
        <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <Focus style={ICON} /> {t('llmApi.imageDetail')}
        </label>
        {imageDetail}
      </div>
    </div>
  );

  return samplingFirst
    ? <>{samplingRow}{imageRow}{requestRow}</>
    : <>{requestRow}{samplingRow}{imageRow}</>;
}
