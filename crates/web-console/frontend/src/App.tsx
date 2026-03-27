import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { SidebarProvider } from '@/components/ui/sidebar';
import { TooltipProvider } from '@/components/ui/tooltip';
import { AppSidebar } from '@/components/app-sidebar';
import { AuthProvider } from '@/components/auth-provider';
import { AuthGuard } from '@/components/auth-guard';

const Login = React.lazy(() =>
  import('@/pages/Login').then((m) => ({ default: m.Login })),
);
const Dashboard = React.lazy(() =>
  import('@/pages/Dashboard').then((m) => ({ default: m.Dashboard })),
);
const Calls = React.lazy(() =>
  import('@/pages/Calls').then((m) => ({ default: m.Calls })),
);
const Agents = React.lazy(() =>
  import('@/pages/Agents').then((m) => ({ default: m.Agents })),
);
const Queues = React.lazy(() =>
  import('@/pages/Queues').then((m) => ({ default: m.Queues })),
);
const Registrations = React.lazy(() =>
  import('@/pages/Registrations').then((m) => ({ default: m.Registrations })),
);
const Health = React.lazy(() =>
  import('@/pages/Health').then((m) => ({ default: m.Health })),
);
const Profile = React.lazy(() =>
  import('@/pages/Profile').then((m) => ({ default: m.Profile })),
);
const UsersPage = React.lazy(() =>
  import('@/pages/Users').then((m) => ({ default: m.Users })),
);
const ApiKeysPage = React.lazy(() =>
  import('@/pages/ApiKeys').then((m) => ({ default: m.ApiKeys })),
);
const RoutingPage = React.lazy(() =>
  import('@/pages/Routing').then((m) => ({ default: m.Routing })),
);
const CallHistoryPage = React.lazy(() =>
  import('@/pages/CallHistory').then((m) => ({ default: m.CallHistory })),
);
const SystemConfigPage = React.lazy(() =>
  import('@/pages/SystemConfig').then((m) => ({ default: m.SystemConfig })),
);
const AuditLogPage = React.lazy(() =>
  import('@/pages/AuditLog').then((m) => ({ default: m.AuditLog })),
);
const PresencePage = React.lazy(() =>
  import('@/pages/Presence').then((m) => ({ default: m.Presence })),
);
const MonitoringPage = React.lazy(() =>
  import('@/pages/Monitoring').then((m) => ({ default: m.Monitoring })),
);
const DepartmentsPage = React.lazy(() =>
  import('@/pages/Departments').then((m) => ({ default: m.Departments })),
);
const SoftphonePage = React.lazy(() =>
  import('@/pages/Softphone').then((m) => ({ default: m.Softphone })),
);
const ExtensionsPage = React.lazy(() =>
  import('@/pages/Extensions').then((m) => ({ default: m.Extensions })),
);
const SkillsPage = React.lazy(() =>
  import('@/pages/Skills').then((m) => ({ default: m.Skills })),
);
const IvrPage = React.lazy(() =>
  import('@/pages/Ivr').then((m) => ({ default: m.Ivr })),
);
const PhoneListsPage = React.lazy(() =>
  import('@/pages/PhoneLists').then((m) => ({ default: m.PhoneLists })),
);
const TrunksPage = React.lazy(() =>
  import('@/pages/Trunks').then((m) => ({ default: m.Trunks })),
);
const SchedulesPage = React.lazy(() =>
  import('@/pages/Schedules').then((m) => ({ default: m.Schedules })),
);
const ReportsPage = React.lazy(() =>
  import('@/pages/Reports').then((m) => ({ default: m.Reports })),
);
const KnowledgePage = React.lazy(() =>
  import('@/pages/Knowledge').then((m) => ({ default: m.Knowledge })),
);
const QualityPage = React.lazy(() =>
  import('@/pages/Quality').then((m) => ({ default: m.Quality })),
);
const AiCopilotPage = React.lazy(() =>
  import('@/pages/AiCopilot').then((m) => ({ default: m.AiCopilot })),
);

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { refetchInterval: 5000, staleTime: 2000 },
  },
});

const suspenseFallback = (
  <div className="flex items-center justify-center h-full">
    <p className="text-muted-foreground">Loading...</p>
  </div>
);

export default function App() {
  return (
    <AuthProvider>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <BrowserRouter>
            <Routes>
              <Route
                path="/login"
                element={
                  <React.Suspense fallback={suspenseFallback}>
                    <Login />
                  </React.Suspense>
                }
              />
              <Route
                path="/*"
                element={
                  <AuthGuard>
                    <SidebarProvider defaultOpen={true}>
                      <div className="flex min-h-svh w-full">
                        <AppSidebar />
                        <main className="flex-1 overflow-auto">
                          <React.Suspense fallback={suspenseFallback}>
                            <Routes>
                              <Route path="/" element={<Dashboard />} />
                              <Route path="/calls" element={<Calls />} />
                              <Route path="/agents" element={<Agents />} />
                              <Route path="/queues" element={<Queues />} />
                              <Route path="/registrations" element={<Registrations />} />
                              <Route path="/health" element={<Health />} />
                              <Route path="/routing" element={<AuthGuard roles={['admin', 'super_admin']}><RoutingPage /></AuthGuard>} />
                              <Route path="/users" element={<AuthGuard roles={['admin', 'super_admin']}><UsersPage /></AuthGuard>} />
                              <Route path="/calls/history" element={<CallHistoryPage />} />
                              <Route path="/system/config" element={<AuthGuard roles={['admin', 'super_admin']}><SystemConfigPage /></AuthGuard>} />
                              <Route path="/system/audit" element={<AuthGuard roles={['admin', 'super_admin']}><AuditLogPage /></AuthGuard>} />
                              <Route path="/presence" element={<PresencePage />} />
                              <Route path="/monitoring" element={<AuthGuard roles={['supervisor', 'admin', 'super_admin']}><MonitoringPage /></AuthGuard>} />
                              <Route path="/departments" element={<AuthGuard roles={['admin', 'super_admin']}><DepartmentsPage /></AuthGuard>} />
                              <Route path="/extensions" element={<AuthGuard roles={['admin', 'super_admin']}><ExtensionsPage /></AuthGuard>} />
                              <Route path="/skills" element={<AuthGuard roles={['admin', 'super_admin']}><SkillsPage /></AuthGuard>} />
                              <Route path="/ivr" element={<AuthGuard roles={['admin', 'super_admin']}><IvrPage /></AuthGuard>} />
                              <Route path="/phone-lists" element={<AuthGuard roles={['admin', 'super_admin']}><PhoneListsPage /></AuthGuard>} />
                              <Route path="/trunks" element={<AuthGuard roles={['super_admin']}><TrunksPage /></AuthGuard>} />
                              <Route path="/schedules" element={<AuthGuard roles={['admin', 'super_admin']}><SchedulesPage /></AuthGuard>} />
                              <Route path="/reports" element={<AuthGuard roles={['supervisor', 'admin', 'super_admin']}><ReportsPage /></AuthGuard>} />
                              <Route path="/quality" element={<AuthGuard roles={['supervisor', 'admin', 'super_admin']}><QualityPage /></AuthGuard>} />
                              <Route path="/knowledge" element={<KnowledgePage />} />
                              <Route path="/softphone" element={<SoftphonePage />} />
                              <Route path="/ai-copilot" element={<AiCopilotPage />} />
                              <Route path="/profile/api-keys" element={<ApiKeysPage />} />
                              <Route path="/profile" element={<Profile />} />
                              <Route path="*" element={<Navigate to="/" replace />} />
                            </Routes>
                          </React.Suspense>
                        </main>
                      </div>
                    </SidebarProvider>
                  </AuthGuard>
                }
              />
            </Routes>
          </BrowserRouter>
        </TooltipProvider>
      </QueryClientProvider>
    </AuthProvider>
  );
}
