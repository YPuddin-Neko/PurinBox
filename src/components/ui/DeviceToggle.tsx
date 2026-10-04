import type { CSSProperties } from 'react';
import { Cpu, Gpu } from 'lucide-react';
import './ui.css';

export interface DeviceToggleProps {
  useGpu: boolean;
  onChange: (useGpu: boolean) => void;
  /** 当前引擎/模型不支持 CPU 时禁用 CPU 按钮 */
  cpuDisabled?: boolean;
  className?: string;
  style?: CSSProperties;
}

const DEVICES = [
  { gpu: false, label: 'CPU', Icon: Cpu, color: '#fbbf24' },
  { gpu: true, label: 'GPU', Icon: Gpu, color: '#4ade80' },
] as const;

export default function DeviceToggle({ useGpu, onChange, cpuDisabled = false, className, style }: DeviceToggleProps) {
  return (
    <div className={className ? `ui-device ${className}` : 'ui-device'} style={style}>
      {DEVICES.map(({ gpu, label, Icon, color }) => (
        <button
          key={label}
          type="button"
          className="ui-toggle ui-device-btn"
          aria-pressed={useGpu === gpu}
          disabled={!gpu && cpuDisabled}
          onClick={() => onChange(gpu)}
          style={{ '--toggle-color': color } as CSSProperties}
        >
          <Icon className="ui-device-icon" /> {label}
        </button>
      ))}
    </div>
  );
}
