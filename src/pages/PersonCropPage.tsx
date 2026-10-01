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
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PersonCropOptions } from '../api/commandOptions';
import Checkbox from '../components/Checkbox';
import ProcessButton from '../components/ProcessButton';
import ProgressLog from '../components/ProgressLog';
import DeviceToggle from '../components/ui/DeviceToggle';
import NumberInput from '../components/ui/NumberInput';
import PageHeader from '../components/ui/PageHeader';
import PathFields from '../components/ui/PathFields';
import { useBatchTask, type ProcessResult } from '../hooks/useBatchTask';
import { hasTauriRuntime, listen } from '../utils/tauriRuntime';

interface CropModelInfo { crop_type: string; downloaded: boolean; }
interface DlProgress { percent: number; speed_mbps: number; status: string; message: string; }

export default function PersonCropPage() {
  const { t } = useTranslation();
  const task = useBatchTask({ event: 'person-crop-progress', taskId: 'person-crop', pythonEnv: true });
  const downloadActive = useRef(false);
  const [inputPath, setInputPath] = useState('');
  const [outputPath, setOutputPath] = useState('');
  const [recursive, setRecursive] = useState(false);
  const [useGpu, setUseGpu] = useState(() => localStorage.getItem('person_crop_gpu') === 'true');
  const [models, setModels] = useState<CropModelInfo[]>([]);
  const [downloading, setDownloading] = useState(false);
  const [personEnabled, setPersonEnabled] = useState(true);
  const [personConf, setPersonConf] = useState(0.3);
  const [upperEnabled, setUpperEnabled] = useState(true);
  const [upperConf, setUpperConf] = useState(0.5);
  const [upperTag, setUpperTag] = useState('upper body');
  const [headEnabled, setHeadEnabled] = useState(true);
  const [headConf, setHeadConf] = useState(0.4);
  const [headTag, setHeadTag] = useState('head view');
  const [headScale, setHeadScale] = useState(1.5);
  const [eyesEnabled, setEyesEnabled] = useState(true);
  const [eyesConf, setEyesConf] = useState(0.3);
  const [eyesTag, setEyesTag] = useState('eyes view');
  const [eyesScale, setEyesScale] = useState(2.4);
  const [keepOriginalTags, setKeepOriginalTags] = useState(false);

  const loadModels = useCallback(async () => {
    if (!hasTauriRuntime()) {
      setModels([]);
      return;
    }
    try { setModels(await invoke<CropModelInfo[]>('get_person_crop_models')); } catch (e) { console.error(e); }
  }, []);

  useEffect(() => { loadModels(); }, []);

  const downloadAll = async () => {
    downloadActive.current = true;
    setDownloading(true);
    task.logger.appendLog(t('personCrop.downloadStart'), 'info');
    try {
      await invoke('download_person_crop_model');
    } catch (e: any) {
      task.logger.appendCatchError(e, t('personCrop.downloadFailed'));
    } finally { downloadActive.current = false; setDownloading(false); }
  };

  useEffect(() => {
    let active = true;
    const p = listen<DlProgress>('person-crop-download', (e) => {
      if (!active || !downloadActive.current) return;
      task.logger.appendDownloadLog(e.payload);
      if (e.payload.status === 'done') loadModels();
    });
    return () => { active = false; p.then(fn => fn()); };
  }, [loadModels, task.logger]);

  const handleProcess = () => {

    return task.run({
      taskName: t('personCrop.taskName'), startLog: t('personCrop.startMsg'), exec: () => invoke<ProcessResult>('start_person_crop', {
        options: {
          input_path: inputPath, output_path: outputPath, use_gpu: useGpu,
          person_enabled: personEnabled, person_conf: personConf,
          upper_enabled: upperEnabled, upper_conf: upperConf, upper_tag: upperTag,
          head_enabled: headEnabled, head_conf: headConf, head_tag: headTag, head_scale: headScale,
          eyes_enabled: eyesEnabled, eyes_conf: eyesConf, eyes_tag: eyesTag, eyes_scale: eyesScale,
          keep_original_tags: keepOriginalTags,
          recursive,
        } satisfies PersonCropOptions,
      })
    });
  };

  const detCard = (
    enabled: boolean, setEnabled: (v: boolean) => void,
    icon: React.ReactNode, label: string, color: string, alphaBase: string,
    modelType: string, children: React.ReactNode,
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
    <div className="page">
      <PageHeader icon={ScanFace} color={'#fb923c'} title={t('personCrop.title')} subtitle={t('personCrop.subtitle')} />

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 'var(--space-6)' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          {/* 路径设置 */}
          <PathFields allowFile title={t('personCrop.cropSettings')} input={inputPath} onInput={setInputPath} output={outputPath} onOutput={setOutputPath} recursive={recursive} onRecursive={setRecursive}>{/* 模型状态 + 下载 */}<div className="form-group">
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
          </div><div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
              <DeviceToggle useGpu={useGpu} onChange={v => { setUseGpu(v); localStorage.setItem('person_crop_gpu', String(v)); }} />
            </div></PathFields>

          {/* 检测选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('personCrop.detOptions')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {detCard(personEnabled, setPersonEnabled, <PersonStanding style={{ width: 18, height: 18 }} />, t('personCrop.fullBody'), '#4ade80', 'rgba(74, 222, 128, ', 'person', <>
                {confSlider(personConf, setPersonConf, '#4ade80')}

              </>)}

              {detCard(upperEnabled, setUpperEnabled, <User style={{ width: 18, height: 18 }} />, t('personCrop.halfBody'), '#818cf8', 'rgba(129, 140, 248, ', 'halfbody', <>
                {confSlider(upperConf, setUpperConf, '#818cf8')}
                {tagInput(upperTag, setUpperTag, 'upper body')}

              </>)}

              {detCard(headEnabled, setHeadEnabled, <CircleUser style={{ width: 18, height: 18 }} />, t('personCrop.headDet'), '#f59e0b', 'rgba(245, 158, 11, ', 'head', <>
                {confSlider(headConf, setHeadConf, '#f59e0b')}
                <div className="form-group" style={{ marginTop: 8, marginBottom: 4 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>{t('personCrop.scaleFactor')}</span>
                    <span style={{ fontFamily: 'monospace', color: '#f59e0b', fontSize: 'var(--font-size-sm)' }}>{headScale.toFixed(1)}x</span>
                  </label>
                  <input type="range" min="1.0" max="3.0" step="0.1" value={headScale} onChange={e => setHeadScale(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#f59e0b' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}><span>1.0x {t('personCrop.headOnly')}</span><span>3.0x {t('personCrop.moreAround')}</span></div>
                </div>
                {tagInput(headTag, setHeadTag, 'head view')}

              </>)}

              {detCard(eyesEnabled, setEyesEnabled, <Eye style={{ width: 18, height: 18 }} />, t('personCrop.eyesDet'), '#f472b6', 'rgba(244, 114, 182, ', 'eyes', <>
                {confSlider(eyesConf, setEyesConf, '#f472b6')}
                <div className="form-group" style={{ marginTop: 8, marginBottom: 4 }}>
                  <label className="form-label" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span>{t('personCrop.scaleFactor')}</span>
                    <span style={{ fontFamily: 'monospace', color: '#f472b6', fontSize: 'var(--font-size-sm)' }}>{eyesScale.toFixed(1)}x</span>
                  </label>
                  <input type="range" min="1.0" max="4.0" step="0.1" value={eyesScale} onChange={e => setEyesScale(Number(e.target.value))}
                    style={{ width: '100%', accentColor: '#f472b6' }} />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 9, color: 'var(--color-text-tertiary)', marginTop: 2 }}><span>1.0x {t('personCrop.eyesOnly')}</span><span>4.0x {t('personCrop.moreAround')}</span></div>
                </div>
                {tagInput(eyesTag, setEyesTag, 'eyes view')}

              </>)}
            </div>
          </div>

          {/* 其他选项 */}
          <div className="tool-panel">
            <div className="tool-panel-header"><span className="tool-panel-title">{t('personCrop.otherOptions')}</span></div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }}>
                <Checkbox checked={keepOriginalTags} onChange={setKeepOriginalTags} color="#7c5cfc" size={16} />
                <span style={{ fontWeight: 600, color: 'var(--color-text-primary)', fontSize: 'var(--font-size-sm)' }}>{t('personCrop.keepTags')}</span>
              </label>

            </div>
          </div>
        </div>

        {/* 右侧 */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-5)' }}>
          <ProcessButton {...task.buttonProps} onStart={handleProcess}
            disabled={!inputPath || !outputPath || !ms.allReady || downloading}
            cancelCommand="cancel_person_crop" forceCancelCommand="force_cancel_person_crop" startText={t('personCrop.startCrop')} />
          <ProgressLog {...task.progressLogProps} />
        </div>
      </div>
    </div>
  );
}
