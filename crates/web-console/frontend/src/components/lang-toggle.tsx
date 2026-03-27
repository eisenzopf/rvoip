import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';

export function LangToggle() {
  const { i18n } = useTranslation();

  const isZh = i18n.language === 'zh';

  function toggle() {
    const next = isZh ? 'en' : 'zh';
    i18n.changeLanguage(next);
    localStorage.setItem('rvoip-lang', next);
  }

  return (
    <Button
      variant="ghost"
      size="sm"
      className="h-7 w-7 text-xs font-bold"
      onClick={toggle}
      aria-label={isZh ? 'Switch to English' : '切换到中文'}
    >
      {isZh ? '中' : 'EN'}
    </Button>
  );
}
