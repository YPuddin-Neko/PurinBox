import { useTranslation } from 'react-i18next';
import RangeField from './ui/RangeField';

interface Props {
  dhash: number;
  onDhash: (value: number) => void;
  phash: number;
  onPhash: (value: number) => void;
  color: number;
  onColor: (value: number) => void;
}

export default function HashThresholdFields({ dhash, onDhash, phash, onPhash, color, onColor }: Props) {
  const { t } = useTranslation();
  const fields = [
    { key: 'dhashThreshold', value: dhash, onChange: onDhash, min: 1, max: 20 },
    { key: 'phashThreshold', value: phash, onChange: onPhash, min: 1, max: 20 },
    { key: 'colorThreshold', value: Math.round(color * 100), onChange: (v: number) => onColor(v / 100), min: 0, max: 100 },
  ];
  return <>{fields.map(field => {
    const isColor = field.max === 100;
    return (
      <RangeField
        key={field.key}
        label={t(`imageDedup.${field.key}`)}
        value={field.value}
        onChange={field.onChange}
        min={field.min}
        max={field.max}
        color="#7c5cfc"
        format={value => (isColor ? color.toFixed(2) : value)}
        ends={[
          `${t(`imageDedup.${isColor ? 'loose' : 'strict'}`)} (${field.min})`,
          `${t(`imageDedup.${isColor ? 'strict' : 'loose'}`)} (${isColor ? '1.0' : field.max})`,
        ]}
        valueStyle={{ fontSize: 13, fontWeight: 700 }}
        className="ui-range-flow"
        style={{ marginBottom: 12 }}
      />
    );
  })}</>;
}
