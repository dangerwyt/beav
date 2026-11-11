import { SettingsPageView } from './settings/SettingsPageView';
import { useSettingsController } from './settings/useSettingsController';

export function Settings(props: Parameters<typeof useSettingsController>[0]) {
  const settings = useSettingsController(props);
  return <SettingsPageView {...settings} />;
}
