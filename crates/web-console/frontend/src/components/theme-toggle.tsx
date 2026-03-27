import { Sun, Moon, Monitor } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useTheme } from '@/hooks/useTheme';

const cycle = { light: 'dark', dark: 'system', system: 'light' } as const;

const icons = {
  light: Sun,
  dark: Moon,
  system: Monitor,
} as const;

const labels = {
  light: 'Switch to dark mode',
  dark: 'Switch to system mode',
  system: 'Switch to light mode',
} as const;

export function ThemeToggle() {
  const { theme, setTheme } = useTheme();

  const Icon = icons[theme];

  return (
    <Button
      variant="ghost"
      size="sm"
      className="h-7 w-7"
      onClick={() => setTheme(cycle[theme])}
      aria-label={labels[theme]}
    >
      <Icon className="size-3.5" />
    </Button>
  );
}
