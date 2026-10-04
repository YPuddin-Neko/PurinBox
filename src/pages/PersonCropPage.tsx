import { invoke } from '@tauri-apps/api/core';
import {
  CircleUser,
  Download,
  Eye,
  Loader2,
  PersonStanding,
  ScanFace,
  User
} from 'lucide-react';
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { buildPersonCropOptions, PERSON_CROP_DEFAULTS } from '../api/commandOptions';
import Checkbox from '../components/Checkbox';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PathFields from '../components/ui/PathFields';
import RangeField from '../components/ui/RangeField';
import ToolPageLayout from '../components/ui/ToolPageLayout';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';
import { hasTauriRuntime } from '../utils/tauriRuntime';

interface CropModelInfo { crop_type: string; downloaded: boolean; }

const GPU_STORAGE_KEY = 'person_crop_gpu';

export default function PersonCropPage() {
  const { t } = useTranslation();
  const task = useBatchTask({
    event: 'person-crop-progress', taskId: 'person-crop', pythonEnv: true,
    download: { event: 'person-crop-download' },
  });
  const d = PERSON_CROP_DEFAULTS;
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [useGpu, setUseGpu] = useState(() => localStorage.getItem(GPU_STORAGE_KEY) === 'true');
  const [models, setModels] = useState<CropModelInfo[]>([]);
  const [downloading, setDownloading] = useState(false);
  const [personEnabled, setPersonEnabled] = useState(d.person_enabled);
  const [personConf, setPersonConf] = useState(d.person_conf);
  const [upperEnabled, setUpperEnabled] = useState(d.upper_enabled);
  const [upperConf, setUpperConf] = useState(d.upper_conf);
  const [upperTag, setUpperTag] = useState(d.upper_tag);
  const [headEnabled, setHeadEnabled] = useState(d.head_enabled);
  const [headConf, setHeadConf] = useState(d.head_conf);
  const [headTag, setHeadTag] = useState(d.head_tag);
  const [headScale, setHeadScale] = useState(d.head_scale);
  const [eyesEnabled, setEyesEnabled] = useState(d.eyes_enabled);
  const [eyesConf, setEyesConf] = useState(d.eyes_conf);
  const [eyesTag, setEyesTag] = useState(d.eyes_tag);
  const [eyesScale, setEyesScale] = useState(d.eyes_scale);
  const [keepOriginalTags, setKeepOriginalTags] = useState(d.keep_original_tags);

  const loadModels = useCallback(async () => {
    if (!hasTauriRuntime()) {
      setModels([]);
      return;
    }
    try { setModels(await invoke<CropModelInfo[]>('get_person_crop_models')); } catch (e) { console.error(e); }
  }, []);

  useEffect(() => { loadModels(); }, []);

  const downloadAll = async () => {
    setDownloading(true);
    task.logger.appendLog(t('personCrop.downloadStart'), 'info');
    try {
      await task.trackDownload(() => invoke('download_person_crop_model'));
    } catch (e) {
      task.logger.appendCatchError(e, t('personCrop.downloadFailed'));
    } finally {
      setDownloading(false);
      loadModels();
    }
  };

  const changeDevice = (gpu: boolean) => {
    setUseGpu(gpu);
    localStorage.setItem(GPU_STORAGE_KEY, String(gpu));
  };

  const handleProcess = () => {
    return task.run({
      taskName: t('personCrop.taskName'),
      startLog: t('personCrop.startMsg'),
      exec: () => invoke<ProcessResult>('start_person_crop', {
        options: buildPersonCropOptions({ input_path: inputPath, output_path: outputPath, recursive }, {
          use_gpu: useGpu,
          person_enabled: personEnabled, person_conf: personConf,
          upper_enabled: upperEnabled, upper_conf: upperConf, upper_tag: upperTag,
          head_enabled: headEnabled, head_conf: headConf, head_tag: headTag, head_scale: headScale,
          eyes_enabled: eyesEnabled, eyes_conf: eyesConf, eyes_tag: eyesTag, eyes_scale: eyesScale,
          keep_original_tags: keepOriginalTags,
        }),
      }),
    });
  };

  const detCard = (
    enabled: boolean, setEnabled: (v: boolean) => void,
    icon: ReactNode, label: string, color: string, alphaBase: string,
    modelType: string, children: ReactNode,
  ) => {
    const m = models.find(x => x.crop_type === modelType);
    return (
      <div style={{ padding: 'var(--space-4)', borderRadius: 'var(--radius-md)', border: `1px solid ${enabled ? `${alphaBase}0.35)` : 'var(--color-border)'}`, background: enabled ? `${alphaBase}0.04)` : 'var(--color-bg-input)', transition: 'all 0.2s' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: enabled ? 'var(--space-3)' : 0 }}>
          <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer', flex: 1 }}>
            <Checkbox checked={enabled} onChange={setEnabled} color={color} size={16} />
            <span style={{ color: enabled ? color : 'var(--color-text-tertiary)' }}>{icon}</span>
            <span style={{ fontWeight: 700, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-md)' }}>{label}</span>
          </label>
          {m && <span style={{ fontSize: 10, padding: '2px 8px', borderRadius: 10, background: m.downloaded ? 'rgba(74,222,128,0.1)' : 'rgba(251,191,36,0.1)', color: m.downloaded ? '#4ade80' : '#fbbf24', fontWeight: 600 }}>{m.downloaded ? t('personCrop.modelReady') : t('personCrop.modelPending')}</span>}
        </div>
        {enabled && <div style={{ paddingLeft: 32 }}>{children}</div>}
      </div>
    );
  };

  const confSlider = (value: number, onChange: (v: number) => void, color: string) => (
    <div className="form-group">
      <label className="form-label">{t('personCrop.confThreshold')}</label>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
        <input type="range" min={0.1} max={0.9} step={0.05} value={value} onChange={e => onChange(Number(e.target.value))} style={{ flex: 1, accentColor: color }} />
        <NumberInput value={value} onChange={onChange} step={0.05} min={0.1} max={0.9} style={{ width: 70, textAlign: 'center' }} />
      </div>
    </div>
  );

  const scaleSlider = (value: number, onChange: (v: number) => void, max: number, color: string, nearEnd: string) => (
    <RangeField label={t('personCrop.scaleFactor')} value={value} onChange={onChange}
      min={1} max={max} step={0.1} color={color} format={v => `${v.toFixed(1)}x`}
      ends={[`1.0x ${nearEnd}`, `${max.toFixed(1)}x ${t('personCrop.moreAround')}`]}
      style={{ marginTop: 8, marginBottom: 4 }} />
  );

  const tagInput = (value: string, onChange: (v: string) => void, placeholder: string) => (
    <div className="form-group">
      <label className="form-label">{t('personCrop.appendTag')}</label>
      <input className="form-input" value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} />
    </div>
  );

  const modelStatusSummary = () => {
    const needed = models.filter(m => {
      if (m.crop_type === 'person') return personEnabled;
      if (m.crop_type === 'halfbody') return upperEnabled;
      if (m.crop_type === 'head') return headEnabled;
      if (m.crop_type === 'eyes') return eyesEnabled;
      return false;
    });
    const ready = needed.filter(m => m.downloaded).length;
    const total = needed.length;
    return { ready, total, allReady: ready === total && total > 0 };
  };

  const ms = modelStatusSummary();

  return (
    <ToolPageLayout icon={ScanFace} color="#fb923c" title={t('personCrop.title')} subtitle={t('personCrop.subtitle')}
      aside={<>
        <ProcessButton {...task.buttonProps} onStart={handleProcess}
          disabled={!inputPath || !outputPath || !ms.allReady || downloading}
          cancelCommand="cancel_person_crop" forceCancelCommand="force_cancel_person_crop" startText={t('personCrop.startCrop')} />
        <ProgressLog {...task.progressLogProps} />
      </>}>
      <PathFields allowFile title={t('personCrop.cropSettings')} input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive}>
        <div className="form-group">
          <label className="form-label">{t('personCrop.detModel')}</label>
          <div style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'center' }}>
            <div style={{ flex: 1, padding: '8px 12px', borderRadius: 'var(--radius-md)', background: 'var(--color-bg-input)', border: '1px solid var(--color-border)', fontSize: 'var(--font-size-sm)', color: 'var(--color-text-secondary)' }}>
              {ms.allReady ? (
                <span style={{ color: '#4ade80' }}>✓ {t('personCrop.allReady')} ({ms.ready}/{ms.total})</span>
              ) : (
                <span style={{ color: '#fbbf24' }}>⬇ {t('personCrop.needDownload')} ({ms.ready}/{ms.total} {t('personCrop.ready')})</span>
              )}
            </div>
            <button className="btn btn-secondary" onClick={downloadAll} disabled={task.processing || downloading || ms.allReady}
              style={{ height: 36, padding: '0 12px', gap: 6, display: 'flex', alignItems: 'center' }}>
              {downloading ? <Loader2 style={{ width: 15, height: 15, animation: 'spin 1s linear infinite' }} /> : <Download style={{ width: 15, height: 15 }} />}
              {downloading ? t('personCrop.downloading') : t('personCrop.downloadModel')}
            </button>
          </div>
          <div style={{ marginTop: 6, fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)' }}>
            {t('personCrop.modelSource')} <a href="https://huggingface.co/deepghs" target="_blank" rel="noreferrer" style={{ color: '#818cf8' }}>deepghs</a>
          </div>
        </div>
        <DeviceToggle useGpu={useGpu} onChange={changeDevice} />
      </PathFields>

      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('personCrop.detOptions')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
          {detCard(personEnabled, setPersonEnabled, <PersonStanding style={{ width: 18, height: 18 }} />, t('personCrop.fullBody'), '#4ade80', 'rgba(74, 222, 128, ', 'person',
            confSlider(personConf, setPersonConf, '#4ade80'))}

          {detCard(upperEnabled, setUpperEnabled, <User style={{ width: 18, height: 18 }} />, t('personCrop.halfBody'), '#818cf8', 'rgba(129, 140, 248, ', 'halfbody', <>
            {confSlider(upperConf, setUpperConf, '#818cf8')}
            {tagInput(upperTag, setUpperTag, d.upper_tag)}
          </>)}

          {detCard(headEnabled, setHeadEnabled, <CircleUser style={{ width: 18, height: 18 }} />, t('personCrop.headDet'), '#f59e0b', 'rgba(245, 158, 11, ', 'head', <>
            {confSlider(headConf, setHeadConf, '#f59e0b')}
            {scaleSlider(headScale, setHeadScale, 3, '#f59e0b', t('personCrop.headOnly'))}
            {tagInput(headTag, setHeadTag, d.head_tag)}
          </>)}

          {detCard(eyesEnabled, setEyesEnabled, <Eye style={{ width: 18, height: 18 }} />, t('personCrop.eyesDet'), '#f472b6', 'rgba(244, 114, 182, ', 'eyes', <>
            {confSlider(eyesConf, setEyesConf, '#f472b6')}
            {scaleSlider(eyesScale, setEyesScale, 4, '#f472b6', t('personCrop.eyesOnly'))}
            {tagInput(eyesTag, setEyesTag, d.eyes_tag)}
          </>)}
        </div>
      </div>

      <div className="tool-panel">
        <div className="tool-panel-header"><span className="tool-panel-title">{t('personCrop.otherOptions')}</span></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
          <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }}>
            <Checkbox checked={keepOriginalTags} onChange={setKeepOriginalTags} color="#7c5cfc" size={16} />
            <span style={{ fontWeight: 600, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-sm)' }}>{t('personCrop.keepTags')}</span>
          </label>
        </div>
      </div>
    </ToolPageLayout>
  );
}
