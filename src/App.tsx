import { Navigate, Route, Routes, useParams } from "react-router-dom";
import { AppShell } from "@/components/layout/AppShell";
import { Dashboard } from "@/routes/Dashboard/Dashboard";
import { Projects } from "@/routes/Projects/Projects";
import { Agents } from "@/routes/Agents/Agents";
import { Tasks } from "@/routes/Tasks/Tasks";
import { Activity } from "@/routes/Activity/Activity";
import { ProjectWorkspaceLayout } from "@/routes/ProjectWorkspace/ProjectWorkspaceLayout";
import { FilesTab } from "@/routes/ProjectWorkspace/FilesTab";
import { ChangesTab } from "@/routes/ProjectWorkspace/ChangesTab";
import { TerminalTab } from "@/routes/ProjectWorkspace/TerminalTab";
import { AgentDetail } from "@/routes/AgentDetail/AgentDetail";
import { Settings } from "@/routes/Settings/Settings";

// ProjectWorkspace's tab components take a `projectId` prop rather than
// reading route params themselves (see FilesTab/ChangesTab/TerminalTab
// docstrings), so these tiny wrappers bridge the route param to the prop.
// They're only ever reached once ProjectWorkspaceLayout finds a real
// project, which never happens in M1.
function FilesTabRoute() {
  const { projectId } = useParams<{ projectId: string }>();
  return <FilesTab projectId={projectId ?? ""} />;
}

function ChangesTabRoute() {
  const { projectId } = useParams<{ projectId: string }>();
  return <ChangesTab projectId={projectId ?? ""} />;
}

function TerminalTabRoute() {
  const { projectId } = useParams<{ projectId: string }>();
  return <TerminalTab projectId={projectId ?? ""} />;
}

export function App() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route path="/" element={<Dashboard />} />
        <Route path="/projects" element={<Projects />} />
        <Route path="/agents" element={<Agents />} />
        <Route path="/tasks" element={<Tasks />} />
        <Route path="/activity" element={<Activity />} />

        <Route path="/projects/:projectId" element={<ProjectWorkspaceLayout />}>
          <Route index element={<Navigate to="files" replace />} />
          <Route path="files" element={<FilesTabRoute />} />
          <Route path="changes" element={<ChangesTabRoute />} />
          <Route path="terminal" element={<TerminalTabRoute />} />
          <Route path="agents/:agentId" element={<AgentDetail />} />
        </Route>

        <Route path="/settings" element={<Settings />} />

        <Route path="*" element={<Navigate to="/" replace />} />
      </Route>
    </Routes>
  );
}
