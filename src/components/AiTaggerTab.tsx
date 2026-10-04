import { useBatchTask } from '../hooks/useBatchTask';
import { notifyTaggerModelsChanged, useTaggerModels } from '../hooks/useTaggerModels';
import { useTaggerJsonSimplified } from '../hooks/useTaggerJsonSimplified';
import { taggerDownloadLog } from '../hooks/useTaggerDownloadLog';
import { TaggerCategoryGrid, ThresholdSliders } from './TaggerControls';
import DeviceToggle from './ui/DeviceToggle';
import NumberInput from './ui/NumberInput';
import { JSON_APPEND_FIELDS, isTaggerCategory, splitOutputFormat, type TagOutputChoice } from '../utils/taggerOptions';
import { JSON_APPEND_FIELD_KEYS, buildTaggerOptions, isOneOf, type JsonAppendField, type TagFileFormat } from '../api/commandOptions';
import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Loader2, Download, Plus, Check, Trash2, Search, FileUp, Save, ChevronDown, X } from 'lucide-react';
import ProgressLog from './ProgressLog';
import ProcessButton from './ProcessButton';
import { ConfirmModal } from './Modal';
import CustomSelect from './CustomSelect';
import TaggerModelSelect from './TaggerModelSelect';
import DatasetPathPanel from './DatasetPathPanel';
import Checkbox from './Checkbox';
import { useTranslation } from 'react-i18next';

interface OnnxModelInfo { input_size: number; input_shape: number[]; }

interface TaggerPreset {
  name: string;
  modelId: string;
  genTh: number;
  charTh: number;
  enabled: string[];
  useGpu: boolean;
  batchSize: number;
  excludeTags: string;
  appendTags: string;
  appendPosition: 'prepend' | 'append';
  jsonAppendField?: string;
  replaceUnderscore: boolean;
  escapeParentheses: boolean;
  sortBy: 'confidence' | 'frequency';
  existingTagsAction: 'overwrite' | 'skip' | 'prepend' | 'append';
  outputFormat: TagFileFormat;
  jsonSimplified: boolean;
}

const DEFAULT_MODEL_SIZE = 448;

export default function AiTaggerTab() {
  const { t } = useTranslation();
  const [inputPath, setInputPath] = useState('');
  const { models, selectedModel, setSelectedModel, genTh, setGenTh, charTh, setCharTh, enabled, setEnabled, cur } = useTaggerModels();
  const task = useBatchTask({ event: 'tagger-progress', taskId: 'tagger', pythonEnv: true, logProcessing: () => true, download: taggerDownloadLog() });
  const taskLogs = task.logger;
  const [useGpu, setUseGpu] = useState(false);
  const [batchSize, setBatchSize] = useState(1);
  const [showAdd, setShowAdd] = useState(false);
  const [nName, setNName] = useState('');
  const [nModelPath, setNModelPath] = useState('');
  const [nTagsPath, setNTagsPath] = useState('');
  const [nSize, setNSize] = useState(DEFAULT_MODEL_SIZE);
  const [detecting, setDetecting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [excludeTags, setExcludeTags] = useState('');
  const [appendTags, setAppendTags] = useState('');
  const [appendPosition, setAppendPosition] = useState<'prepend' | 'append'>('append');
  const [jsonAppendField, setJsonAppendField] = useState<JsonAppendField>('tags');
  const [replaceUnderscore, setReplaceUnderscore] = useState(true);
  const [escapeParentheses, setEscapeParentheses] = useState(false);
  const [sortBy, setSortBy] = useState<'confidence' | 'frequency'>('confidence');
  const [existingTagsAction, setExistingTagsAction] = useState<'overwrite' | 'skip' | 'prepend' | 'append'>('overwrite');
  const [outputFormat, setOutputFormat] = useState<TagFileFormat>('txt');
  const [jsonSimplified, setJsonSimplified] = useTaggerJsonSimplified();
  const outputChoice: TagOutputChoice = outputFormat === 'txt' ? 'txt' : jsonSimplified ? 'json_simplified' : 'json';
  const [recursive, setRecursive] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState<{ id: string; name: string } | null>(null);

  const [presets, setPresets] = useState<TaggerPreset[]>(() => {
    try { return JSON.parse(localStorage.getItem('tagger_presets') || '[]'); } catch { return []; }
  });
  const [showPresets, setShowPresets] = useState(false);
  const [presetInput, setPresetInput] = useState('');
  const [showPresetSave, setShowPresetSave] = useState(false);
  const presetRef = useRef<HTMLDivElement>(null);

  // 点击外部关闭预设下拉
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (presetRef.current && !presetRef.current.contains(e.target as Node)) {
        setShowPresets(false);
        setShowPresetSave(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, []);

  const savePresetsToStorage = (list: TaggerPreset[]) => {
    setPresets(list);
    localStorage.setItem('tagger_presets', JSON.stringify(list));
  };

  const handleSavePreset = () => {
    const name = presetInput.trim();
    if (!name) return;
    const preset: TaggerPreset = {
      name,
      modelId: selectedModel,
      genTh, charTh,
      enabled: Array.from(enabled),
      useGpu,
      batchSize,
      excludeTags, appendTags, appendPosition, jsonAppendField,
      replaceUnderscore, escapeParentheses,
      sortBy, existingTagsAction,
      outputFormat, jsonSimplified,
    };
    // 同名覆盖
    const next = presets.filter(p => p.name !== name);
    next.unshift(preset);
    savePresetsToStorage(next);
    setPresetInput('');
    setShowPresetSave(false);
    taskLogs.appendLog(t('aiTagger.presetSaved', { name }), 'success');
  };

  const handleLoadPreset = (preset: TaggerPreset) => {
    setSelectedModel(preset.modelId);
    setGenTh(preset.genTh);
    setCharTh(preset.charTh);
    setEnabled(new Set(preset.enabled.filter(isTaggerCategory)));
    setUseGpu(preset.useGpu);
    setBatchSize(preset.batchSize ?? 1);
    setExcludeTags(preset.excludeTags);
    setAppendTags(preset.appendTags);
    setAppendPosition(preset.appendPosition);
    setJsonAppendField(isOneOf(JSON_APPEND_FIELD_KEYS, preset.jsonAppendField) ? preset.jsonAppendField : 'tags');
    setReplaceUnderscore(preset.replaceUnderscore);
    setEscapeParentheses(preset.escapeParentheses ?? false);
    setSortBy(preset.sortBy ?? 'confidence');
    setExistingTagsAction(preset.existingTagsAction ?? 'overwrite');
    setOutputFormat(preset.outputFormat);
    setJsonSimplified(preset.jsonSimplified);
    setShowPresets(false);
    taskLogs.appendLog(t('aiTagger.presetLoaded', { name: preset.name }), 'info');
  };

  const handleDeletePreset = (name: string) => {
    savePresetsToStorage(presets.filter(p => p.name !== name));
  };

  const handleStart = async () => {
    if (!inputPath || !selectedModel || enabled.size === 0) return;
    await task.run({
      taskName: `${t('aiTagger.taskName')} - ${cur?.name || '?'}`,
      startLog: t('aiTagger.startMsg', { model: cur?.name, hw: useGpu ? 'GPU' : 'CPU' }),
      exec: async () => {
        try {
          return await invoke('start_tagging', {
            options: buildTaggerOptions({ input_path: inputPath, recursive }, {
              model_id: selectedModel,
              general_threshold: genTh,
              character_threshold: charTh,
              enabled_categories: [...enabled],
              use_gpu: useGpu,
              batch_size: batchSize,
              exclude_tags: excludeTags,
              append_tags: appendTags,
              append_position: appendPosition,
              json_append_field: jsonAppendField,
              replace_underscore: replaceUnderscore,
              escape_parentheses: escapeParentheses,
              sort_by: sortBy,
              existing_tags_action: existingTagsAction,
              output_format: outputFormat,
              json_simplified: jsonSimplified,
            }),
          });
        } finally {
          // 打标时可能下载了模型
          notifyTaggerModelsChanged();
        }
      },
    });
  };

  const browseOnnx = async () => {
    const f = await open({ multiple: false, filters: [{ name: 'ONNX Model', extensions: ['onnx'] }] });
    if (f) setNModelPath(f as string);
  };
  const browseTags = async () => {
    const f = await open({ multiple: false, filters: [{ name: t('aiTagger.tagFileLabel'), extensions: ['csv', 'json'] }] });
    if (f) setNTagsPath(f as string);
  };

  const autoDetect = async () => {
    if (!nModelPath) { taskLogs.appendLog(t('aiTagger.selectOnnxFirst'), 'error'); return; }
    setDetecting(true);
    try {
      const info = await invoke<OnnxModelInfo>('detect_onnx_model_info', { modelPath: nModelPath });
      setNSize(info.input_size);
      taskLogs.appendLog(t('aiTagger.detectOk', { size: info.input_size, shape: info.input_shape.join(', ') }), 'success');
    } catch (e: any) {
      taskLogs.appendLog(`${t('aiTagger.detectFail')}: ${String(e)}`, 'error');
    }
    setDetecting(false);
  };

  const handleImport = async () => {
    if (!nName || !nModelPath || !nTagsPath) {
      taskLogs.appendLog(t('aiTagger.fillAllFields'), 'error');
      return;
    }
    setImporting(true);
    try {
      await invoke<string>('import_local_tagger_model', { name: nName, modelPath: nModelPath, tagsPath: nTagsPath, inputSize: nSize });
      taskLogs.appendLog(t('aiTagger.importOk', { name: nName }), 'success');
      setShowAdd(false); setNName(''); setNModelPath(''); setNTagsPath(''); setNSize(DEFAULT_MODEL_SIZE);
      notifyTaggerModelsChanged();
    } catch (e: any) {
      taskLogs.appendLog(`${t('aiTagger.importFail')}: ${String(e)}`, 'error');
    }
    setImporting(false);
  };

  const handleDelete = async (id: string, name: string) => {
    try {
      await invoke('remove_custom_tagger_model', { id });
      taskLogs.appendLog(`${t('aiTagger.deletedModel')}: ${name}`, 'info');
      notifyTaggerModelsChanged();
    } catch (e: any) {
      taskLogs.appendLog(`${t('aiTagger.deleteFail')}: ${String(e)}`, 'error');
    }
  };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-5)', alignItems: 'start' }}>
      {/* 左栏 - 所有设置 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <DatasetPathPanel value={inputPath} onChange={setInputPath} recursive={recursive} onRecursive={setRecursive} />

        {/* 打标模型 */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('aiTagger.taggerModel')}</span>
            <div style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'center' }}>
              {/* 配置预设按钮 */}
              <div style={{ position: 'relative' }} ref={presetRef}>
                <button className="btn btn-ghost btn-sm" onClick={() => { setShowPresets(!showPresets); setShowPresetSave(false); }} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                  <Save style={{ width: 13, height: 13 }} /> {t('aiTagger.presets')} <ChevronDown style={{ width: 12, height: 12 }} />
                </button>
                {showPresets && (
                  <div style={{ position: 'absolute', top: '100%', right: 0, marginTop: 4, minWidth: 220, background: 'var(--color-bg-elevated)', border: '1px solid var(--color-border)', borderRadius: 'var(--radius-md)', boxShadow: '0 8px 24px rgba(0,0,0,0.2)', zIndex: 100, overflow: 'hidden' }}>
                    {/* 保存当前配置 */}
                    {!showPresetSave ? (
                      <button onClick={() => setShowPresetSave(true)} style={{ width: '100%', padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 6, border: 'none', background: 'transparent', color: 'var(--color-accent-primary)', cursor: 'pointer', fontSize: 12, fontWeight: 600, borderBottom: '1px solid var(--color-border)' }}>
                        <Plus style={{ width: 12, height: 12 }} /> {t('aiTagger.savePreset')}
                      </button>
                    ) : (
                      <div style={{ padding: '8px 10px', display: 'flex', gap: 4, borderBottom: '1px solid var(--color-border)', alignItems: 'center' }}>
                        <input className="form-input" placeholder={t('aiTagger.presetName')} value={presetInput} onChange={e => setPresetInput(e.target.value)} onKeyDown={e => { if (e.key === 'Enter') handleSavePreset(); }} style={{ flex: 1, fontSize: 11, padding: '4px 8px' }} autoFocus />
                        <button className="btn btn-primary btn-sm" onClick={handleSavePreset} disabled={!presetInput.trim()} style={{ padding: '4px 8px', fontSize: 11 }}><Check style={{ width: 11, height: 11 }} /></button>
                        <button className="btn btn-ghost btn-sm" onClick={() => setShowPresetSave(false)} style={{ padding: '4px 6px' }}><X style={{ width: 11, height: 11 }} /></button>
                      </div>
                    )}
                    {/* 预设列表 */}
                    <div style={{ maxHeight: 200, overflowY: 'auto' }}>
                      {presets.length === 0 ? (
                        <div style={{ padding: '12px', fontSize: 11, color: 'var(--color-text-tertiary)', textAlign: 'center' }}>{t('aiTagger.noPresets')}</div>
                      ) : presets.map(p => (
                        <div key={p.name} style={{ display: 'flex', alignItems: 'center', padding: '6px 12px', cursor: 'pointer', fontSize: 12, gap: 6, borderBottom: '1px solid rgba(127,127,127,0.08)' }}
                          onClick={() => handleLoadPreset(p)}
                          onMouseEnter={e => (e.currentTarget.style.background = 'rgba(124,92,252,0.06)')}
                          onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}>
                          <span style={{ flex: 1, fontWeight: 500 }}>{p.name}</span>
                          <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)' }}>{models.find(m => m.id === p.modelId)?.name || '?'}</span>
                          <button onClick={e => { e.stopPropagation(); handleDeletePreset(p.name); }} style={{ background: 'none', border: 'none', cursor: 'pointer', padding: '2px', color: '#f87171', display: 'flex' }}><Trash2 style={{ width: 11, height: 11 }} /></button>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
              <button className="btn btn-ghost btn-sm" onClick={() => setShowAdd(!showAdd)}>
                {showAdd ? t('aiTagger.close') : <><Plus style={{ width: 14, height: 14 }} /> {t('aiTagger.importModel')}</>}
              </button>
            </div>
          </div>
          {showAdd && (
            <div style={{ padding: 'var(--space-3)', borderRadius: 'var(--radius-md)', background: 'rgba(124,92,252,0.04)', border: '1px solid rgba(124,92,252,0.15)', marginBottom: 'var(--space-3)', display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
              <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.nameLabel')}</label><input className="form-input" placeholder={t('aiTagger.namePlaceholder')} value={nName} onChange={e => setNName(e.target.value)} style={{ width: '100%' }} /></div>
              <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.modelFile')}</label><div style={{ display: 'flex', gap: 'var(--space-2)' }}><input className="form-input" placeholder={t('aiTagger.modelPlaceholder')} value={nModelPath} onChange={e => setNModelPath(e.target.value)} style={{ flex: 1 }} readOnly /><button className="btn btn-secondary btn-sm" onClick={browseOnnx}><FileUp style={{ width: 14, height: 14 }} /> {t('aiTagger.browse')}</button></div></div>
              <div><label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.tagMapping')}</label><div style={{ display: 'flex', gap: 'var(--space-2)' }}><input className="form-input" placeholder={t('aiTagger.tagFilePlaceholder')} value={nTagsPath} onChange={e => setNTagsPath(e.target.value)} style={{ flex: 1 }} readOnly /><button className="btn btn-secondary btn-sm" onClick={browseTags}><FileUp style={{ width: 14, height: 14 }} /> {t('aiTagger.browse')}</button></div></div>
              <div style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'flex-end' }}>
                <div>
                  <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.inputSize')}</label>
                  <NumberInput integer min={1} fallback={DEFAULT_MODEL_SIZE} value={nSize} onChange={setNSize} style={{ width: 80 }} />
                </div>
                <button className="btn btn-secondary btn-sm" onClick={autoDetect} disabled={detecting || !nModelPath} style={{ height: 34, whiteSpace: 'nowrap' }}>{detecting ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <Search style={{ width: 14, height: 14 }} />} {t('aiTagger.autoDetect')}</button>
                <button className="btn btn-primary btn-sm" onClick={handleImport} disabled={importing || !nName || !nModelPath || !nTagsPath} style={{ height: 34 }}>{importing ? <Loader2 style={{ width: 14, height: 14, animation: 'spin 1s linear infinite' }} /> : <Plus style={{ width: 14, height: 14 }} />} {t('aiTagger.add')}</button>
              </div>
            </div>
          )}
          <TaggerModelSelect models={models} value={selectedModel} onChange={setSelectedModel}
            formatLabel={m => `${m.name} ${m.is_downloaded ? '✓' : '⬇'}`} />
          {cur?.requires_token && <div style={{ fontSize: 11, marginTop: 6, color: 'var(--color-warning)' }}>{t('aiTagger.requiresToken')}</div>}
          {cur && (
            <div style={{ marginTop: 'var(--space-2)', fontSize: 'var(--font-size-xs)', color: 'var(--color-text-tertiary)', display: 'flex', gap: 'var(--space-3)', alignItems: 'center' }}>
              <span style={{ flexShrink: 0, whiteSpace: 'nowrap', padding: '1px 6px', borderRadius: 'var(--radius-full)', fontSize: 10, background: cur.is_downloaded ? 'rgba(74,222,128,0.1)' : 'rgba(251,191,36,0.1)', color: cur.is_downloaded ? '#4ade80' : '#fbbf24' }}>{cur.is_downloaded ? t('aiTagger.downloaded') : t('aiTagger.toDownload')}</span>
              <span style={{ flexShrink: 0, whiteSpace: 'nowrap', padding: '1px 6px', borderRadius: 'var(--radius-full)', fontSize: 10, background: 'rgba(124,92,252,0.1)', color: '#a78bfa' }}>{cur.input_size}px</span>
              {!cur.is_builtin && (<button className="btn btn-ghost btn-sm" onClick={() => setDeleteConfirm({ id: cur.id, name: cur.name })} style={{ marginLeft: 'auto', padding: '2px 6px', color: '#f87171' }}><Trash2 style={{ width: 12, height: 12 }} /> {t('aiTagger.deleteModel')}</button>)}
            </div>
          )}
        </div>

        {/* 标签分类与阈值 */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">{t('aiTagger.catAndThreshold')}</span></div>
          <TaggerCategoryGrid enabled={enabled} onChange={setEnabled} supported={cur?.supported_categories} />
          <ThresholdSliders general={genTh} character={charTh} onGeneral={setGenTh} onCharacter={setCharTh} />
        </div>

        {/* 其他设置（含 GPU/CPU 开关） */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('aiTagger.otherSettings')}</span>
            {/* GPU/CPU 切换 */}
            <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
              <span style={{ fontSize: 11, fontWeight: 600, color: useGpu ? 'var(--color-text-secondary)' : 'var(--color-text-tertiary)', whiteSpace: 'nowrap' }}>{t('aiTagger.batchSize')}</span>
              <NumberInput integer min={1} max={64} fallback={1} value={batchSize} onChange={setBatchSize} disabled={!useGpu}
                style={{ width: 58, padding: '3px 6px', fontSize: 11, textAlign: 'center', opacity: useGpu ? 1 : 0.4 }} />
              <div style={{ width: 1, height: 16, background: 'var(--color-border)', margin: '0 4px' }} />
              <DeviceToggle useGpu={useGpu} onChange={setUseGpu} />
            </div>
          </div>
          {/* 复选框行 */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-4)', padding: '8px 10px', marginBottom: 'var(--space-3)' }}>
            <Checkbox checked={replaceUnderscore} onChange={setReplaceUnderscore} label={t('aiTagger.replaceUnderscore')} style={{ gap: 8, fontSize: 12, fontWeight: 600 }} />
            <Checkbox checked={escapeParentheses} onChange={setEscapeParentheses} label={t('aiTagger.escapeParentheses')} style={{ gap: 8, fontSize: 12, fontWeight: 600 }} />
          </div>
          {/* 输出格式 + 已标识文件操作 + 标签顺序 - 三列同行 */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 'var(--space-3)', marginBottom: 'var(--space-3)' }}>
            <div>
              <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.outputFormat')}</label>
              <CustomSelect value={outputChoice}
                onChange={v => {
                  const { output_format, json_simplified } = splitOutputFormat(v as TagOutputChoice);
                  setOutputFormat(output_format);
                  if (output_format === 'json') setJsonSimplified(json_simplified);
                }}
                options={[
                  { value: 'txt', label: '.txt' },
                  { value: 'json', label: `.json (${t('aiTagger.fullFormat')})` },
                  { value: 'json_simplified', label: `.json (${t('aiTagger.simplified')})` },
                ] satisfies { value: TagOutputChoice; label: string }[]} compact />
            </div>
            <div>
              <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.existingTagsAction')}</label>
              <CustomSelect value={existingTagsAction}
                onChange={v => setExistingTagsAction(v as 'overwrite' | 'skip' | 'prepend' | 'append')}
                options={[
                  { value: 'overwrite', label: t('aiTagger.existingAction_overwrite') },
                  { value: 'skip', label: t('aiTagger.existingAction_skip') },
                  { value: 'prepend', label: t('aiTagger.existingAction_prepend') },
                  { value: 'append', label: t('aiTagger.existingAction_append') },
                ]} compact />
            </div>
            <div>
              <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.sortBy')}</label>
              <CustomSelect value={sortBy}
                onChange={v => setSortBy(v as 'confidence' | 'frequency')}
                options={[
                  { value: 'confidence', label: t('aiTagger.sortBy_confidence') },
                  { value: 'frequency', label: t('aiTagger.sortBy_frequency') },
                ]} compact />
            </div>
          </div>
          {/* 排除标签 */}
          <div style={{ marginBottom: 'var(--space-3)' }}>
            <label className="form-label" style={{ fontSize: 11, marginBottom: 4 }}>{t('aiTagger.excludeTags')}</label>
            <input className="form-input" placeholder="tag1, tag2, tag3 ..." value={excludeTags} onChange={e => setExcludeTags(e.target.value)} style={{ width: '100%' }} />
          </div>
          {/* 额外追加标签 */}
          <div>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
              <label className="form-label" style={{ fontSize: 11, margin: 0 }}>{t('aiTagger.appendTags')}</label>
              <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
                {outputFormat === 'json' && (
                  <div style={{ width: 120 }} title={t('aiTagger.appendField')}>
                    <CustomSelect value={jsonAppendField}
                      onChange={v => { if (isOneOf(JSON_APPEND_FIELD_KEYS, v)) setJsonAppendField(v); }}
                      options={JSON_APPEND_FIELDS.map(f => ({ value: f.value, label: t(f.labelKey) }))} compact />
                  </div>
                )}
                {(['prepend', 'append'] as const).map(pos => (
                  <button key={pos} onClick={() => setAppendPosition(pos)}
                    style={{ padding: '2px 8px', borderRadius: 'var(--radius-sm)', border: `1px solid ${appendPosition === pos ? 'var(--color-border-active)' : 'var(--color-border)'}`,
                      background: appendPosition === pos ? 'rgba(124,92,252,0.08)' : 'transparent',
                      color: appendPosition === pos ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)', fontSize: 10, fontWeight: 600, cursor: 'pointer' }}>
                    {pos === 'prepend' ? t('aiTagger.prepend') : t('aiTagger.append')}
                  </button>
                ))}
              </div>
            </div>
            <input className="form-input" placeholder="tag1, tag2, tag3 ..." value={appendTags} onChange={e => setAppendTags(e.target.value)} style={{ width: '100%' }} />
            <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 4 }}>{t('aiTagger.appendTagsTip')}</div>
          </div>
        </div>
      </div>

      {/* 右栏 - 操作 + 进度 + 日志 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
        <ProcessButton {...task.buttonProps} onStart={handleStart}
          disabled={!inputPath || !selectedModel || enabled.size === 0}
          cancelCommand="cancel_tagging" forceCancelCommand="force_cancel_tagging"
          startText={cur && !cur.is_downloaded ? t('aiTagger.downloadAndTag') : t('aiTagger.startTag')}
          startIcon={cur && !cur.is_downloaded ? <Download style={{ width: 18, height: 18 }} /> : undefined}
          processingText={t('aiTagger.tagging')}
          />


        <ProgressLog {...task.progressLogProps} />
      </div>

      <ConfirmModal
        open={!!deleteConfirm}
        onClose={() => setDeleteConfirm(null)}
        onConfirm={() => { if (deleteConfirm) handleDelete(deleteConfirm.id, deleteConfirm.name); }}
        title={t('aiTagger.deleteTitle')}
        message={t('aiTagger.deleteMsg', { name: deleteConfirm?.name })}
        confirmText={t('aiTagger.deleteConfirm')}
        variant="error"
      />
    </div>
  );
}
