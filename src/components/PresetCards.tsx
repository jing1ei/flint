/** The four presets as radio cards — the top of the settings drawer and the only thing most
 * people should ever touch. Selecting one replaces the whole `Settings` object server-side. */
import { useStore } from "../state/store";

export function PresetCards(): React.JSX.Element | null {
  const presets = useStore((s) => s.catalog?.presets);
  const current = useStore((s) => s.settings?.preset);
  const choose = useStore((s) => s.choosePreset);

  if (presets === undefined) return null;

  return (
    <fieldset className="presets">
      <legend className="presets__legend">Preset</legend>
      {presets.map((preset) => (
        <label key={preset.id} className="preset" data-active={preset.id === current || undefined}>
          <input
            type="radio"
            name="preset"
            value={preset.id}
            checked={preset.id === current}
            onChange={() => void choose(preset.id)}
          />
          <span className="preset__body">
            <span className="preset__label">{preset.label}</span>
            <span className="preset__desc">{preset.description}</span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}
