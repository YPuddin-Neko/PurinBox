import { useEffect, useId, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Loader2, Save } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { SaveSdMetadataArgs, SdMetadataDocument } from '../api/commandOptions';
import { errorText } from '../utils/tauriRuntime';
import { getMetadataFields, updateMetadataField, validateMetadataText, type MetadataField } from '../utils/sdMetadataEditor';

interface Props {
  path: string;
  onSaved: (document: SdMetadataDocument) => void;
  onCancel: () => void;
  onStateChange: (state: { dirty: boolean; busy: boolean }) => void;
}

function FieldInput({ field, disabled, onChange }: {
  field: MetadataField; disabled: boolean; onChange: (value: string) => string | null;
}) {
  const { t } = useTranslation();
  const id = useId();
  const [value, setValue] = useState(field.value);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setValue(field.value); setError(null); }, [field.value]);
  const change = (next: string) => { setValue(next); setError(onChange(next)); };
  const label = field.label.startsWith('sdMetadata.') ? t(field.label) : field.label;
  const shared = { id, disabled, value, 'aria-invalid': !!error, 'aria-describedby': error ? `${id}-error` : undefined, className: 'form-input' };
  return <div className={`metadata-edit-field${field.multiline ? ' metadata-edit-field-wide' : ''}`}>
    <label className="form-label" htmlFor={id}>{label}</label>
    {field.kind === 'boolean' ? <select {...shared} onChange={e => change(e.target.value)}>
      <option value="true">true</option><option value="false">false</option>
    </select> : field.multiline ? <textarea {...shared} rows={3} onChange={e => change(e.target.value)} />
      : <input {...shared} type="text" inputMode={field.kind === 'number' ? 'decimal' : undefined} onChange={e => change(e.target.value)} />}
    {error && <span id={`${id}-error`} className="metadata-edit-error">{error}</span>}
  </div>;
}

export default function SdMetadataEditor({ path, onSaved, onCancel, onStateChange }: Props) {
  const { t } = useTranslation();
  const entryId = useId();
  const rawId = useId();
  const [document, setDocument] = useState<SdMetadataDocument | null>(null);
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const [entryIndex, setEntryIndex] = useState<number>(0);
  const [raw, setRaw] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [reload, setReload] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [invalidFields, setInvalidFields] = useState<Record<string, string>>({});
  useEffect(() => {
    let active = true;
    setLoading(true); setError(null);
    void invoke<SdMetadataDocument>('read_sd_metadata_document', { filePath: path }).then(result => {
      if (!active) return;
      setDocument(result);
      setDrafts(Object.fromEntries(result.entries.map(entry => [entry.index, entry.text])));
      setEntryIndex(result.entries[0]?.index ?? 0);
    }).catch(err => { if (active) setError(errorText(err)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [path, reload]);

  const changes = document?.entries.filter(entry => drafts[entry.index] !== entry.text) ?? [];
  const dirty = changes.length > 0 || Object.keys(invalidFields).length > 0;
  const busy = loading || saving;
  useEffect(() => { onStateChange({ dirty, busy: saving }); }, [dirty, saving, onStateChange]);
  const entry = document?.entries.find(item => item.index === entryIndex);
  const text = entry ? drafts[entry.index] ?? entry.text : '';
  const fields = useMemo(() => entry ? getMetadataFields(entry.keyword, text) : [], [entry, text]);
  const groups = [...new Set(fields.map(field => field.group ?? ''))];
  const fullParams = fields.find(field => field.id === 'a1111:params');

  const updateField = (field: MetadataField, value: string) => {
    if (!entry) return null;
    const key = `${entry.index}:${field.id}`;
    try {
      const next = updateMetadataField(entry.keyword, text, field.id, value);
      const nextFields = getMetadataFields(entry.keyword, next);
      setDrafts(previous => ({ ...previous, [entry.index]: next }));
      setInvalidFields(previous => {
        const nextErrors = { ...previous };
        delete nextErrors[key];
        for (const prior of fields) {
          const updated = nextFields.find(candidate => candidate.id === prior.id);
          if (!updated || updated.value !== prior.value) delete nextErrors[`${entry.index}:${prior.id}`];
        }
        return nextErrors;
      });
      setError(null);
      return null;
    } catch (err) {
      const message = errorText(err);
      const translated = message.startsWith('sdMetadata.') ? t(message) : message;
      setInvalidFields(previous => ({ ...previous, [key]: translated }));
      return translated;
    }
  };

  const save = async () => {
    if (!document || busy || !dirty || Object.keys(invalidFields).length) return;
    setError(null);
    try {
      for (const changed of changes) validateMetadataText(changed.keyword, drafts[changed.index], changed.text);
    } catch (err) {
      const message = errorText(err);
      setError(message.startsWith('sdMetadata.') ? t(message) : message);
      return;
    }
    setSaving(true);
    try {
      const result = await invoke<SdMetadataDocument>('save_sd_metadata_document', {
        filePath: document.path, revision: document.revision,
        edits: changes.map(changed => ({ index: changed.index, text: drafts[changed.index] })),
      } satisfies SaveSdMetadataArgs);
      onSaved(result);
    } catch (err) { setError(errorText(err)); }
    finally { setSaving(false); }
  };

  return <div className="metadata-editor" aria-busy={busy}>
    {loading ? <div role="status" className="metadata-edit-status"><Loader2 size={16} className="metadata-spin" />{t('sdMetadata.loadingEditor')}</div>
      : document ? <>
        <div className="metadata-edit-toolbar">
          <label htmlFor={entryId} className="form-label">{t('sdMetadata.metadataField')}</label>
          <select id={entryId} className="form-input" value={entryIndex} disabled={saving || Object.keys(invalidFields).length > 0}
            onChange={e => { setEntryIndex(Number(e.target.value)); setError(null); }}>
            {document.entries.map(item => <option key={item.index} value={item.index}>{item.keyword}{document.entries.filter(other => other.keyword === item.keyword).length > 1 ? ` (${item.index})` : ''}</option>)}
          </select>
          {fields.length > 0 && <button className="btn btn-secondary btn-sm" disabled={saving || Object.keys(invalidFields).length > 0}
            onClick={() => setRaw(!raw)}>{t(raw ? 'sdMetadata.editFields' : 'sdMetadata.editRaw')}</button>}
        </div>
        {entry?.keyword === 'prompt' && document.entries.some(item => item.keyword === 'workflow') && <span className="metadata-edit-hint">{t('sdMetadata.comfyWorkflowHint')}</span>}
        {entry && (raw || fields.length === 0 ? <div className="metadata-edit-field">
          <label className="form-label" htmlFor={rawId}>{entry.keyword}</label>
          <textarea id={rawId} className="form-input metadata-edit-raw" rows={14} value={text} disabled={saving} spellCheck={false}
            onChange={e => { setDrafts(previous => ({ ...previous, [entry.index]: e.target.value })); setError(null); }} />
        </div> : groups.map(group => <div key={group} className="metadata-edit-group">
          {group && <div className="metadata-edit-group-title">{group.startsWith('sdMetadata.') ? t(group) : group === 'positive' || group === 'negative' ? t(`sdMetadata.${group}`) : group}</div>}
          <div className="metadata-edit-fields">{fields.filter(field => (field.group ?? '') === group && field.id !== 'a1111:params').map(field =>
            <FieldInput key={`${entry.index}:${field.id}`} field={field} disabled={saving} onChange={value => updateField(field, value)} />
          )}</div>
        </div>))}
        {!raw && fullParams && <details className="metadata-edit-full-params">
          <summary>{t('sdMetadata.fullParams')}</summary>
          <FieldInput field={fullParams} disabled={saving} onChange={value => updateField(fullParams, value)} />
        </details>}
        {document.entries.length === 0 && <span>{t('sdMetadata.noEditableMetadata')}</span>}
      </> : null}
    {error && <div role="alert" className="metadata-edit-error">{error}</div>}
    {!loading && !document && <button className="btn btn-secondary btn-sm" onClick={() => setReload(value => value + 1)}>{t('sdMetadata.retryRead')}</button>}
    <div className="metadata-edit-footer">
      <span className="metadata-edit-save-note">{t('sdMetadata.saveToImage')}</span>
      <button className="btn btn-secondary btn-sm" disabled={saving} onClick={onCancel}>{t('modal.cancel')}</button>
      <button className="btn btn-primary btn-sm" disabled={busy || !dirty || Object.keys(invalidFields).length > 0} onClick={() => void save()}>
        {saving ? <Loader2 size={14} className="metadata-spin" /> : <Save size={14} />}{t(saving ? 'sdMetadata.savingMetadata' : 'sdMetadata.saveMetadata')}
      </button>
    </div>
  </div>;
}
