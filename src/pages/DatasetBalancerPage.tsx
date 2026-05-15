import { useState, useMemo } from 'react';
import { Scale, Plus, Trash2, RotateCcw, LayoutGrid, PieChart as PieIcon, AlignLeft, AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import CustomSelect from '../components/CustomSelect';

interface ConceptFolder { id: number; name: string; imageCount: number; repeats: number; }
const COLORS = ['#7c5cfc', '#f59e0b', '#4ade80', '#38bdf8', '#f87171', '#a78bfa', '#fb923c', '#2dd4bf', '#e879f9', '#facc15'];
let nextId = 1;
const mkFolder = (n?: string): ConceptFolder => ({ id: nextId++, name: n || `concept_${nextId - 1}`, imageCount: 20, repeats: 1 });

type VizMode = 'treemap' | 'pie' | 'timeline';

export default function DatasetBalancerPage() {
  const { t } = useTranslation();
  const [folders, setFolders] = useState<ConceptFolder[]>([mkFolder('character'), mkFolder('outfit')]);
  const [mode, setMode] = useState<'by_epoch' | 'by_steps'>('by_epoch');
  const [batchSize, setBatchSize] = useState(1);
  const [gradAccum, setGradAccum] = useState(1);
  const [epochs, setEpochs] = useState(10);
  const [maxSteps, setMaxSteps] = useState(2000);
  const [vizMode, setVizMode] = useState<VizMode>('treemap');
  const [hovered, setHovered] = useState<number | null>(null);

  const addFolder = () => setFolders(p => [...p, mkFolder()]);
  const rmFolder = (id: number) => setFolders(p => p.filter(f => f.id !== id));
  const updFolder = (id: number, k: keyof ConceptFolder, v: any) => setFolders(p => p.map(f => f.id === id ? { ...f, [k]: v } : f));
  const resetAll = () => { nextId = 1; setFolders([mkFolder('character'), mkFolder('outfit')]); setBatchSize(1); setGradAccum(1); setEpochs(10); setMaxSteps(2000); setMode('by_epoch'); };

  const calc = useMemo(() => {
    const eb = batchSize * gradAccum;
    const details = folders.map((f, i) => {
      const samples = f.imageCount * f.repeats;
      return { ...f, samples, color: COLORS[i % COLORS.length] };
    });
    const totalSamples = details.reduce((s, d) => s + d.samples, 0);
    const stepsPerEpoch = eb > 0 ? Math.ceil(totalSamples / eb) : 0;
    const withPct = details.map(d => ({ ...d, percent: totalSamples > 0 ? d.samples / totalSamples * 100 : 0 }));

    if (mode === 'by_epoch') {
      return { stepsPerEpoch, totalSteps: stepsPerEpoch * epochs, totalSamples, folders: withPct, eb, epochs, fullEpochs: epochs, remaining: 0, suggestedSteps: 0 };
    }
    const fullEpochs = stepsPerEpoch > 0 ? Math.floor(maxSteps / stepsPerEpoch) : 0;
    const remaining = stepsPerEpoch > 0 ? maxSteps - fullEpochs * stepsPerEpoch : 0;
    const computedEpochs = stepsPerEpoch > 0 ? Math.ceil(maxSteps / stepsPerEpoch) : 0;
    const suggestedSteps = computedEpochs * stepsPerEpoch;
    return { stepsPerEpoch, totalSteps: maxSteps, totalSamples, folders: withPct, eb, epochs: computedEpochs, fullEpochs, remaining, suggestedSteps };
  }, [folders, batchSize, gradAccum, epochs, maxSteps, mode]);

  const isCut = mode === 'by_steps' && calc.remaining > 0;

  // === Viz toggle button ===
  const VizBtn = ({ m, icon }: { m: VizMode; icon: React.ReactNode }) => (
    <button onClick={() => setVizMode(m)} style={{
      padding: '4px 8px', borderRadius: 'var(--radius-sm)', border: `1px solid ${vizMode === m ? 'var(--color-border-active)' : 'var(--color-border)'}`,
      background: vizMode === m ? 'rgba(124,92,252,0.08)' : 'transparent', cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 4,
      color: vizMode === m ? 'var(--color-accent-primary)' : 'var(--color-text-tertiary)', fontSize: 11, fontWeight: 600, fontFamily: 'inherit',
    }}>{icon}</button>
  );

  // === Treemap ===
  const renderTreemap = () => (
    <div style={{ display: 'flex', height: 220, borderRadius: 'var(--radius-md)', overflow: 'hidden', gap: 2 }}>
      {calc.folders.map((f, i) => (
        <div key={f.id} onMouseEnter={() => setHovered(i)} onMouseLeave={() => setHovered(null)} style={{
          flex: f.percent, background: f.color, position: 'relative', display: 'flex', flexDirection: 'column',
          justifyContent: 'flex-end', padding: 'var(--space-3)', transition: 'all 0.3s', cursor: 'pointer',
          opacity: hovered === null || hovered === i ? 1 : 0.55, filter: hovered === i ? 'brightness(1.1)' : 'none',
          minWidth: 40,
        }}>
          <div style={{ color: '#fff', fontWeight: 700, fontSize: f.percent > 15 ? 14 : 11, textShadow: '0 1px 4px rgba(0,0,0,0.4)' }}>{f.name}</div>
          <div style={{ color: 'rgba(255,255,255,0.8)', fontSize: 12, fontFamily: 'monospace', textShadow: '0 1px 3px rgba(0,0,0,0.3)' }}>{f.percent.toFixed(1)}%</div>
          {/* Hover tooltip */}
          {hovered === i && (
            <div style={{
              position: 'absolute', top: 8, left: '50%', transform: 'translateX(-50%)', padding: '8px 12px',
              background: 'rgba(0,0,0,0.85)', borderRadius: 'var(--radius-md)', color: '#fff', fontSize: 11,
              whiteSpace: 'nowrap', zIndex: 10, pointerEvents: 'none', lineHeight: 1.6,
              boxShadow: '0 4px 16px rgba(0,0,0,0.3)',
            }}>
              <div style={{ fontWeight: 700, marginBottom: 2 }}>{f.name}</div>
              <div>{t('datasetBalancer.imageCount')}: {f.imageCount} × {f.repeats} = {f.samples}</div>
              <div>{t('datasetBalancer.proportion')}: {f.percent.toFixed(1)}%</div>
            </div>
          )}
        </div>
      ))}
    </div>
  );

  // === Pie Chart (SVG donut) ===
  const renderPie = () => {
    const cx = 110, cy = 110, r = 95, ir = 55;
    let startAngle = -Math.PI / 2;
    const segs = calc.folders.map((f, i) => {
      const angle = (f.percent / 100) * Math.PI * 2;
      const end = startAngle + angle;
      const la = angle > Math.PI ? 1 : 0;
      const path = [
        `M ${cx + r * Math.cos(startAngle)} ${cy + r * Math.sin(startAngle)}`,
        `A ${r} ${r} 0 ${la} 1 ${cx + r * Math.cos(end)} ${cy + r * Math.sin(end)}`,
        `L ${cx + ir * Math.cos(end)} ${cy + ir * Math.sin(end)}`,
        `A ${ir} ${ir} 0 ${la} 0 ${cx + ir * Math.cos(startAngle)} ${cy + ir * Math.sin(startAngle)}`,
        'Z',
      ].join(' ');
      const mid = startAngle + angle / 2;
      const seg = { ...f, path, mid, idx: i };
      startAngle = end;
      return seg;
    });
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 'var(--space-5)', height: 220 }}>
        <svg viewBox="0 0 220 220" style={{ width: 220, height: 220 }}>
          {segs.map((s, i) => (
            <path key={i} d={s.path} fill={s.color} stroke="var(--color-bg-card)" strokeWidth={2}
              opacity={hovered === null || hovered === i ? 1 : 0.4}
              onMouseEnter={() => setHovered(i)} onMouseLeave={() => setHovered(null)}
              style={{ transition: 'opacity 0.2s, transform 0.2s', cursor: 'pointer',
                transformOrigin: `${cx}px ${cy}px`,
                transform: hovered === i ? `translate(${Math.cos(s.mid) * 6}px,${Math.sin(s.mid) * 6}px)` : 'none',
              }} />
          ))}
          <text x={cx} y={cy - 6} textAnchor="middle" fill="var(--color-text-secondary)" fontSize={11} fontWeight={600}>{t('datasetBalancer.totalSamples')}</text>
          <text x={cx} y={cy + 14} textAnchor="middle" fill="var(--color-text-primary)" fontSize={20} fontWeight={700} fontFamily="monospace">{calc.totalSamples}</text>
        </svg>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {calc.folders.map((f, i) => (
            <div key={f.id} onMouseEnter={() => setHovered(i)} onMouseLeave={() => setHovered(null)}
              style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer', opacity: hovered === null || hovered === i ? 1 : 0.5, transition: 'opacity 0.2s' }}>
              <div style={{ width: 10, height: 10, borderRadius: 2, background: f.color, flexShrink: 0 }} />
              <span style={{ fontSize: 12, color: 'var(--color-text-primary)', fontWeight: 600 }}>{f.name}</span>
              <span style={{ fontSize: 11, color: 'var(--color-text-tertiary)', fontFamily: 'monospace' }}>{f.percent.toFixed(1)}%</span>
            </div>
          ))}
        </div>
      </div>
    );
  };

  // === Timeline (by_steps only) ===
  const renderTimeline = () => {
    const totalEpochs = calc.epochs;
    const spe = calc.stepsPerEpoch;
    const totalUsed = totalEpochs * spe;
    const cutPos = spe > 0 ? (maxSteps / totalUsed) * 100 : 100;
    const stepsNeeded = spe - calc.remaining;
    return (
      <div style={{ height: 220, display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 'var(--space-3)' }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--color-text-secondary)', textAlign: 'center' }}>{t('datasetBalancer.trainingProgress')}</div>
        <div style={{ position: 'relative', padding: '0 20px' }}>
          {/* Epoch blocks */}
          <div style={{ display: 'flex', gap: 2, position: 'relative' }}>
            {Array.from({ length: totalEpochs }).map((_, i) => {
              const isLast = i === totalEpochs - 1 && isCut;
              return (
                <div key={i} style={{
                  flex: 1, height: 40, borderRadius: 3,
                  background: isLast ? 'linear-gradient(90deg, rgba(124,92,252,0.25), rgba(248,113,113,0.15))' : 'rgba(124,92,252,0.25)',
                  border: isLast ? '1px dashed rgba(248,113,113,0.4)' : '1px solid rgba(124,92,252,0.15)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: totalEpochs > 20 ? 8 : 10, color: 'var(--color-text-tertiary)', fontFamily: 'monospace',
                }}>
                  {totalEpochs <= 30 && `E${i + 1}`}
                </div>
              );
            })}
            {/* Cutoff line */}
            {isCut && (
              <div style={{
                position: 'absolute', left: `${cutPos}%`, top: -12, bottom: -20,
                borderLeft: '2px dashed #f87171', zIndex: 2,
              }}>
                <div style={{
                  position: 'absolute', top: -2, left: 6, fontSize: 10, color: '#f87171',
                  whiteSpace: 'nowrap', fontWeight: 600, fontFamily: 'monospace',
                }}>MAX_TRAIN_STEPS={maxSteps}</div>
              </div>
            )}
          </div>
          {/* Lost area label */}
          {isCut && (
            <div style={{ textAlign: 'right', fontSize: 10, color: '#f87171', marginTop: 4, fontFamily: 'monospace' }}>
              {t('datasetBalancer.lostSteps', { steps: stepsNeeded })}
            </div>
          )}
        </div>
        {/* Warning */}
        {isCut && (
          <div style={{
            margin: '0 20px', padding: 'var(--space-3)', borderRadius: 'var(--radius-md)',
            background: 'rgba(251,191,36,0.06)', border: '1px solid rgba(251,191,36,0.2)',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 4 }}>
              <AlertTriangle style={{ width: 14, height: 14, color: '#fbbf24', flexShrink: 0 }} />
              <span style={{ fontSize: 12, fontWeight: 700, color: '#fbbf24' }}>
                {t('datasetBalancer.warningTitle', { epoch: calc.fullEpochs + 1 })}
              </span>
            </div>
            <div style={{ fontSize: 11, color: 'var(--color-text-secondary)', lineHeight: 1.6 }}>
              {t('datasetBalancer.warningBody', { steps: spe - calc.remaining, suggested: calc.suggestedSteps })}
            </div>
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Scale style={{ width: 28, height: 28, color: '#f59e0b' }} />
          <h1 className="page-title">{t('datasetBalancer.title')}</h1>
        </div>
        <p className="page-subtitle">{t('datasetBalancer.subtitle')}</p>
      </div>

      {/* Row 1: Visualization */}
      <div className="tool-panel" style={{ marginBottom: 'var(--space-4)' }}>
        <div className="tool-panel-header">
          <span className="tool-panel-title">{t('datasetBalancer.balancePreview')}</span>
          <div style={{ display: 'flex', gap: 4 }}>
            <VizBtn m="treemap" icon={<LayoutGrid style={{ width: 14, height: 14 }} />} />
            <VizBtn m="pie" icon={<PieIcon style={{ width: 14, height: 14 }} />} />
            {mode === 'by_steps' && <VizBtn m="timeline" icon={<AlignLeft style={{ width: 14, height: 14 }} />} />}
          </div>
        </div>
        {vizMode === 'treemap' && renderTreemap()}
        {vizMode === 'pie' && renderPie()}
        {vizMode === 'timeline' && mode === 'by_steps' && renderTimeline()}
        {vizMode === 'timeline' && mode !== 'by_steps' && renderTreemap()}
      </div>

      {/* Row 2: Three columns */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 'var(--space-4)', alignItems: 'start' }}>
        {/* Col 1: Concept Folders */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('datasetBalancer.conceptFolders')}</span>
            <button className="btn btn-ghost btn-sm" onClick={addFolder}><Plus style={{ width: 14, height: 14 }} /> {t('datasetBalancer.addFolder')}</button>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-2)', maxHeight: 300, overflow: 'auto' }}>
            {folders.map((f, idx) => (
              <div key={f.id} style={{
                display: 'grid', gridTemplateColumns: '1fr 70px 70px 28px', gap: 6, alignItems: 'end',
                padding: '6px 8px', borderRadius: 'var(--radius-sm)',
                background: 'var(--color-bg-input)', border: '1px solid var(--color-border)',
              }}>
                <div>
                  <div style={{ fontSize: 9, color: COLORS[idx % COLORS.length], marginBottom: 2, fontWeight: 700 }}>
                    {t('datasetBalancer.folderLabel', { index: idx + 1 })}
                  </div>
                  <input className="form-input" value={f.name} onChange={e => updFolder(f.id, 'name', e.target.value)}
                    style={{ width: '100%', fontSize: 11, padding: '3px 6px', height: 26 }} />
                </div>
                <div>
                  <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginBottom: 2 }}>{t('datasetBalancer.imageCount')}</div>
                  <input className="form-input" type="number" min={1} value={f.imageCount}
                    onChange={e => updFolder(f.id, 'imageCount', Math.max(1, Number(e.target.value)))}
                    style={{ width: '100%', fontSize: 11, padding: '3px 4px', height: 26, textAlign: 'center' }} />
                </div>
                <div>
                  <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginBottom: 2 }}>{t('datasetBalancer.repeats')}</div>
                  <input className="form-input" type="number" min={1} value={f.repeats}
                    onChange={e => updFolder(f.id, 'repeats', Math.max(1, Number(e.target.value)))}
                    style={{ width: '100%', fontSize: 11, padding: '3px 4px', height: 26, textAlign: 'center' }} />
                </div>
                <button className="btn btn-ghost btn-sm" onClick={() => rmFolder(f.id)} disabled={folders.length <= 1}
                  style={{ padding: 2, opacity: folders.length <= 1 ? 0.3 : 1, height: 26 }}>
                  <Trash2 style={{ width: 12, height: 12, color: '#f87171' }} />
                </button>
              </div>
            ))}
          </div>
        </div>

        {/* Col 2: Training Params */}
        <div className="tool-panel">
          <div className="tool-panel-header">
            <span className="tool-panel-title">{t('datasetBalancer.trainingParams')}</span>
            <button className="btn btn-ghost btn-sm" onClick={resetAll}><RotateCcw style={{ width: 14, height: 14 }} /> {t('datasetBalancer.reset')}</button>
          </div>
          <div style={{ marginBottom: 'var(--space-3)' }}>
            <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 4 }}>{t('datasetBalancer.calcMode')}</div>
            <CustomSelect value={mode} onChange={v => { setMode(v as any); if (v === 'by_epoch' && vizMode === 'timeline') setVizMode('treemap'); }} options={[
              { value: 'by_epoch', label: t('datasetBalancer.modeEpoch') },
              { value: 'by_steps', label: t('datasetBalancer.modeSteps') },
            ]} />
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-3)' }}>
            <div>
              <label className="form-label" style={{ fontSize: 10, marginBottom: 4, display: 'flex', justifyContent: 'space-between' }}>
                <span>Batch Size</span><span style={{ fontWeight: 700, color: '#a78bfa', fontFamily: 'monospace' }}>{batchSize}</span>
              </label>
              <input type="range" min={1} max={256} value={batchSize} onChange={e => setBatchSize(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
            <div>
              <label className="form-label" style={{ fontSize: 10, marginBottom: 4, display: 'flex', justifyContent: 'space-between' }}>
                <span>Grad Accum</span><span style={{ fontWeight: 700, color: '#a78bfa', fontFamily: 'monospace' }}>{gradAccum}</span>
              </label>
              <input type="range" min={1} max={256} value={gradAccum} onChange={e => setGradAccum(Number(e.target.value))} style={{ width: '100%', accentColor: 'var(--color-accent-primary)' }} />
            </div>
            {mode === 'by_epoch' ? (
              <div>
                <label className="form-label" style={{ fontSize: 10, marginBottom: 4 }}>Epochs</label>
                <input className="form-input" type="number" min={1} value={epochs} onChange={e => setEpochs(Math.max(1, Number(e.target.value)))} style={{ width: '100%' }} />
              </div>
            ) : (
              <div>
                <label className="form-label" style={{ fontSize: 10, marginBottom: 4 }}>Max Steps</label>
                <input className="form-input" type="number" min={1} value={maxSteps} onChange={e => setMaxSteps(Math.max(1, Number(e.target.value)))} style={{ width: '100%' }} />
              </div>
            )}
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 6,
              borderRadius: 'var(--radius-md)', background: 'rgba(124,92,252,0.04)', border: '1px solid rgba(124,92,252,0.12)' }}>
              <div style={{ textAlign: 'center' }}>
                <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)' }}>{t('datasetBalancer.effectiveBatch')}</div>
                <div style={{ fontSize: 16, fontWeight: 700, fontFamily: 'monospace', color: '#a78bfa' }}>{calc.eb}</div>
              </div>
            </div>
          </div>
        </div>

        {/* Col 3: Results */}
        <div className="tool-panel">
          <div className="tool-panel-header"><span className="tool-panel-title">{t('datasetBalancer.results')}</span></div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--space-2)' }}>
            {[
              { label: t('datasetBalancer.stepsPerEpoch'), value: calc.stepsPerEpoch, color: '#a78bfa' },
              { label: t('datasetBalancer.totalSteps'), value: calc.totalSteps, color: '#f59e0b' },
              { label: t('datasetBalancer.totalSamples'), value: calc.totalSamples, color: '#4ade80' },
              { label: mode === 'by_steps' ? t('datasetBalancer.computedEpochs') : 'Epochs', value: calc.epochs, color: '#38bdf8' },
            ].map((m, i) => (
              <div key={i} style={{ padding: 'var(--space-2)', borderRadius: 'var(--radius-md)',
                background: `${m.color}08`, border: `1px solid ${m.color}20`, textAlign: 'center' }}>
                <div style={{ fontSize: 9, color: 'var(--color-text-tertiary)', marginBottom: 2 }}>{m.label}</div>
                <div style={{ fontSize: 18, fontWeight: 700, fontFamily: 'monospace', color: m.color }}>{m.value}</div>
              </div>
            ))}
          </div>
          {/* Distribution mini */}
          <div style={{ marginTop: 'var(--space-3)' }}>
            <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginBottom: 4 }}>{t('datasetBalancer.distribution')}</div>
            {calc.folders.map((f) => (
              <div key={f.id} style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 4 }}>
                <div style={{ width: 8, height: 8, borderRadius: 2, background: f.color, flexShrink: 0 }} />
                <span style={{ fontSize: 11, flex: 1, color: 'var(--color-text-primary)' }}>{f.name}</span>
                <span style={{ fontSize: 10, color: 'var(--color-text-tertiary)', fontFamily: 'monospace' }}>{f.imageCount}×{f.repeats}={f.samples}</span>
                <span style={{ fontSize: 10, fontWeight: 700, fontFamily: 'monospace', color: f.color, minWidth: 36, textAlign: 'right' }}>{f.percent.toFixed(1)}%</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
