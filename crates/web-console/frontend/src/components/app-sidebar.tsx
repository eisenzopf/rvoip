import { useMemo } from 'react';
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarFooter,
} from '@/components/ui/sidebar';
import {
  LayoutDashboard,
  Phone,
  Users,
  ListOrdered,
  Radio,
  HeartPulse,
  LogOut,
  UserCircle,
  UsersRound,
  KeyRound,
  Route,
  Clock,
  Wrench,
  FileText,
  CircleDot,
  Monitor,
  Building2,
  Headphones,
  PhoneCall,
  Zap,
  Mic,
  ShieldBan,
  Cable,
  CalendarDays,
  BarChart3,
  BookOpen,
  ClipboardCheck,
  BrainCircuit,
} from 'lucide-react';
import { useLocation, Link } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ThemeToggle } from '@/components/theme-toggle';
import { LangToggle } from '@/components/lang-toggle';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { useAuth } from '@/hooks/useAuth';

interface NavItem {
  titleKey: string;
  icon: React.ComponentType<{ className?: string }>;
  href: string;
  /** Minimum roles required to see this item. Empty = all roles can see. */
  minRoles?: string[];
}

interface NavGroup {
  labelKey: string;
  items: NavItem[];
  /** Minimum roles required to see this group. Empty = all roles can see. */
  minRoles?: string[];
}

const allNavGroups: NavGroup[] = [
  {
    labelKey: 'nav.overview',
    items: [
      { titleKey: 'nav.dashboard', icon: LayoutDashboard, href: '/' },
      { titleKey: 'nav.presence', icon: CircleDot, href: '/presence' },
      { titleKey: 'nav.monitoring', icon: Monitor, href: '/monitoring', minRoles: ['supervisor', 'admin', 'super_admin'] },
    ],
  },
  {
    labelKey: 'nav.callCenter',
    minRoles: ['supervisor', 'admin', 'super_admin'],
    items: [
      { titleKey: 'nav.activeCalls', icon: Phone, href: '/calls' },
      { titleKey: 'nav.callHistory', icon: Clock, href: '/calls/history' },
      { titleKey: 'nav.queues', icon: ListOrdered, href: '/queues' },
      { titleKey: 'nav.agents', icon: Users, href: '/agents' },
      { titleKey: 'nav.ivr', icon: Mic, href: '/ivr', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.phoneLists', icon: ShieldBan, href: '/phone-lists', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.reports', icon: BarChart3, href: '/reports', minRoles: ['supervisor', 'admin', 'super_admin'] },
      { titleKey: 'nav.quality', icon: ClipboardCheck, href: '/quality', minRoles: ['supervisor', 'admin', 'super_admin'] },
    ],
  },
  {
    labelKey: 'nav.sip',
    minRoles: ['supervisor', 'admin', 'super_admin'],
    items: [
      { titleKey: 'nav.registrations', icon: Radio, href: '/registrations' },
    ],
  },
  {
    labelKey: 'nav.organization',
    minRoles: ['admin', 'super_admin'],
    items: [
      { titleKey: 'nav.departments', icon: Building2, href: '/departments', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.extensions', icon: PhoneCall, href: '/extensions', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.skills', icon: Zap, href: '/skills', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.schedules', icon: CalendarDays, href: '/schedules', minRoles: ['admin', 'super_admin'] },
    ],
  },
  {
    labelKey: 'nav.qualityManagement',
    items: [
      { titleKey: 'nav.knowledge', icon: BookOpen, href: '/knowledge' },
      { titleKey: 'nav.quality', icon: ClipboardCheck, href: '/quality', minRoles: ['supervisor', 'admin', 'super_admin'] },
    ],
  },
  {
    labelKey: 'nav.tools',
    items: [
      { titleKey: 'nav.softphone', icon: Headphones, href: '/softphone' },
      { titleKey: 'nav.copilot', icon: BrainCircuit, href: '/ai-copilot' },
    ],
  },
  {
    labelKey: 'nav.system',
    items: [
      { titleKey: 'nav.trunks', icon: Cable, href: '/trunks', minRoles: ['super_admin'] },
      { titleKey: 'nav.users', icon: UsersRound, href: '/users', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.routing', icon: Route, href: '/routing', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.systemConfig', icon: Wrench, href: '/system/config', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.audit', icon: FileText, href: '/system/audit', minRoles: ['admin', 'super_admin'] },
      { titleKey: 'nav.health', icon: HeartPulse, href: '/health', minRoles: ['supervisor', 'admin', 'super_admin'] },
      { titleKey: 'nav.apiKeys', icon: KeyRound, href: '/profile/api-keys', minRoles: ['supervisor', 'admin', 'super_admin'] },
      { titleKey: 'nav.profile', icon: UserCircle, href: '/profile' },
    ],
  },
];

export function AppSidebar() {
  const location = useLocation();
  const { t } = useTranslation();
  const { user, logout, hasAnyRole } = useAuth();

  const filteredGroups = useMemo(() => {
    return allNavGroups
      .filter((group) => {
        if (!group.minRoles || group.minRoles.length === 0) return true;
        return hasAnyRole(group.minRoles);
      })
      .map((group) => ({
        ...group,
        items: group.items.filter((item) => {
          if (!item.minRoles || item.minRoles.length === 0) return true;
          return hasAnyRole(item.minRoles);
        }),
      }))
      .filter((group) => group.items.length > 0);
  }, [hasAnyRole]);

  return (
    <Sidebar>
      <SidebarHeader className="border-b px-4 py-3">
        <div className="flex items-center gap-2">
          <div className="flex h-7 w-7 items-center justify-center rounded-md bg-primary text-primary-foreground text-xs font-bold">
            rv
          </div>
          <span className="font-semibold text-sm tracking-tight">rvoip</span>
          <span className="ml-auto rounded bg-muted px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground">
            v0.1.26
          </span>
        </div>
      </SidebarHeader>

      <SidebarContent>
        {filteredGroups.map((group) => (
          <SidebarGroup key={group.labelKey}>
            <SidebarGroupLabel>{t(group.labelKey)}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {group.items.map((item) => (
                  <SidebarMenuItem key={item.titleKey}>
                    <SidebarMenuButton
                      isActive={location.pathname === item.href}
                      render={<Link to={item.href} />}
                    >
                      <item.icon className="size-4" />
                      <span>{t(item.titleKey)}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>

      <SidebarFooter className="border-t px-4 py-3 space-y-2">
        {user && (
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 min-w-0">
              <UserCircle className="size-4 shrink-0 text-muted-foreground" />
              <span className="text-sm font-medium truncate">{user.username}</span>
              {user.roles[0] && (
                <Badge variant="secondary" className="text-[10px]">{user.roles[0]}</Badge>
              )}
            </div>
            <Button variant="ghost" size="icon-xs" onClick={logout} title={t('sidebar.logout')}>
              <LogOut className="size-3.5" />
            </Button>
          </div>
        )}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <div className="h-2 w-2 rounded-full bg-green-500 animate-pulse" />
            {t('sidebar.allSystemsOperational')}
          </div>
          <div className="flex items-center gap-1">
            <LangToggle />
            <ThemeToggle />
          </div>
        </div>
      </SidebarFooter>
    </Sidebar>
  );
}
