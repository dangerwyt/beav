import type { Dispatch } from 'react';
import { useCallback, useRef, useState, type SetStateAction } from 'react';
import { appAlert, appConfirm } from "../../utils/appDialogs";
import { type Advisor } from "../Advisors";
import { sortTeamAdvisors } from './settingsSupport';

export function useSettingsTeam({ setTestMsg, setStatus }: {
  setTestMsg: Dispatch<SetStateAction<string>>;
  setStatus: Dispatch<SetStateAction<"error" | "idle" | "saving" | "saved">>;
}) {

  const [teamAdvisors, setTeamAdvisors] = useState<Advisor[]>([]);
  const [isTeamAdvisorsLoading, setIsTeamAdvisorsLoading] = useState(false);
  const [teamAdvisorBusyId, setTeamAdvisorBusyId] = useState<string | null>(null);
  const [isCreatingTeamAdvisor, setIsCreatingTeamAdvisor] = useState(false);
  const [editingTeamAdvisor, setEditingTeamAdvisor] = useState<Advisor | null>(null);
  const [settingsTeamAdvisor, setSettingsTeamAdvisor] = useState<Advisor | null>(null);
  const [draggingTeamAdvisorId, setDraggingTeamAdvisorId] = useState<string | null>(null);
  const [isTeamSystemPromptExpanded, setIsTeamSystemPromptExpanded] = useState(false);
  const [isTeamOptimizingPrompt, setIsTeamOptimizingPrompt] = useState(false);

  const teamAdvisorOrderRef = useRef<Advisor[]>([]);

  const teamAdvisorDragIdRef = useRef<string | null>(null);

  const loadTeamAdvisors = useCallback(async () => {
    setIsTeamAdvisorsLoading(true);
    try {
      const list = await window.ipcRenderer.advisors.list<Advisor>();
      const sorted = sortTeamAdvisors(Array.isArray(list) ? list : []);
      setTeamAdvisors(sorted);
      teamAdvisorOrderRef.current = sorted;
      return sorted;
    } catch (error) {
      console.error('Failed to load team advisors:', error);
      setTestMsg('成员列表读取失败');
      setStatus('error');
      return [];
    } finally {
      setIsTeamAdvisorsLoading(false);
    }
  }, []);

  const persistTeamAdvisorOrder = useCallback(async (items: Advisor[]) => {
    await Promise.all(items.map((advisor, index) => (
      window.ipcRenderer.advisors.update({
        id: advisor.id,
        redclawOrder: index,
        redclawVisible: advisor.redclawVisible !== false,
      })
    )));
    teamAdvisorOrderRef.current = items;
    window.dispatchEvent(new Event('redclaw:team-settings-changed'));
  }, []);

  const handleToggleTeamAdvisorVisible = useCallback((advisor: Advisor) => {
    const nextVisible = advisor.redclawVisible === false;
    setTeamAdvisorBusyId(advisor.id);
    setTeamAdvisors((prev) => prev.map((item) => (
      item.id === advisor.id ? { ...item, redclawVisible: nextVisible } : item
    )));
    void window.ipcRenderer.advisors.update({
      id: advisor.id,
      redclawVisible: nextVisible,
    }).then(() => {
      window.dispatchEvent(new Event('redclaw:team-settings-changed'));
    }).catch((error) => {
      console.error('Failed to update advisor visibility:', error);
      setTeamAdvisors((prev) => prev.map((item) => (
        item.id === advisor.id ? { ...item, redclawVisible: advisor.redclawVisible } : item
      )));
      setTestMsg('成员展示设置保存失败');
      setStatus('error');
    }).finally(() => {
      setTeamAdvisorBusyId(null);
    });
  }, []);

  const handleTeamAdvisorDragStart = useCallback((advisorId: string) => {
    teamAdvisorDragIdRef.current = advisorId;
    setDraggingTeamAdvisorId(advisorId);
  }, []);

  const handleTeamAdvisorDragOver = useCallback((targetAdvisorId: string) => {
    setTeamAdvisors((prev) => {
      const draggingId = teamAdvisorDragIdRef.current;
      if (!draggingId || draggingId === targetAdvisorId) return prev;
      const fromIndex = prev.findIndex((item) => item.id === draggingId);
      const toIndex = prev.findIndex((item) => item.id === targetAdvisorId);
      if (fromIndex < 0 || toIndex < 0) return prev;
      const next = [...prev];
      const [moved] = next.splice(fromIndex, 1);
      next.splice(toIndex, 0, moved);
      const ordered = next.map((item, index) => ({ ...item, redclawOrder: index }));
      teamAdvisorOrderRef.current = ordered;
      return ordered;
    });
  }, []);

  const handleTeamAdvisorDragEnd = useCallback(() => {
    if (!teamAdvisorDragIdRef.current) return;
    teamAdvisorDragIdRef.current = null;
    setDraggingTeamAdvisorId(null);
    void persistTeamAdvisorOrder(teamAdvisorOrderRef.current).catch((error) => {
      console.error('Failed to persist advisor order:', error);
      setTestMsg('成员排序保存失败');
      setStatus('error');
      void loadTeamAdvisors();
    });
  }, [loadTeamAdvisors, persistTeamAdvisorOrder]);

  const refreshTeamAdvisor = useCallback(async (advisorId: string) => {
    const list = await loadTeamAdvisors();
    const updated = list.find((item) => item.id === advisorId) || null;
    if (updated) {
      setSettingsTeamAdvisor(updated);
      setEditingTeamAdvisor((prev) => prev?.id === advisorId ? updated : prev);
    }
    return updated;
  }, [loadTeamAdvisors]);

  const handleOpenTeamAdvisorSettings = useCallback((advisor: Advisor) => {
    setSettingsTeamAdvisor(advisor);
    setIsTeamSystemPromptExpanded(false);
  }, []);

  const handleCreateTeamAdvisor = useCallback(() => {
    setEditingTeamAdvisor(null);
    setIsCreatingTeamAdvisor(true);
  }, []);

  const handleDeleteTeamAdvisor = useCallback(async (advisor: Advisor) => {
    if (!(await appConfirm('确定要删除这个智囊团成员吗？', { title: '删除成员', confirmLabel: '删除', tone: 'danger' }))) return;
    try {
      await window.ipcRenderer.advisors.delete(advisor.id);
      setSettingsTeamAdvisor(null);
      setEditingTeamAdvisor(null);
      await loadTeamAdvisors();
      window.dispatchEvent(new Event('redclaw:team-settings-changed'));
    } catch (error) {
      console.error('Failed to delete advisor:', error);
      setTestMsg('成员删除失败');
      setStatus('error');
    }
  }, [loadTeamAdvisors]);

  const handleUploadTeamAdvisorKnowledge = useCallback(async (advisor: Advisor) => {
    try {
      await window.ipcRenderer.advisors.uploadKnowledge(advisor.id);
      await refreshTeamAdvisor(advisor.id);
    } catch (error) {
      console.error('Failed to upload advisor knowledge:', error);
      setTestMsg('知识库上传失败');
      setStatus('error');
    }
  }, [refreshTeamAdvisor]);

  const handleDeleteTeamAdvisorKnowledge = useCallback(async (advisor: Advisor, fileName: string) => {
    if (!(await appConfirm(`确定要删除知识库文件 "${fileName}" 吗？`, { title: '删除知识文件', confirmLabel: '删除', tone: 'danger' }))) return;
    try {
      await window.ipcRenderer.advisors.deleteKnowledge({ advisorId: advisor.id, fileName });
      await refreshTeamAdvisor(advisor.id);
    } catch (error) {
      console.error('Failed to delete advisor knowledge:', error);
      setTestMsg('知识文件删除失败');
      setStatus('error');
    }
  }, [refreshTeamAdvisor]);

  const handleOptimizeTeamAdvisorPrompt = useCallback(async (advisor: Advisor) => {
    setIsTeamOptimizingPrompt(true);
    try {
      const result = await window.ipcRenderer.advisors.optimizePromptDeep({
        advisorId: advisor.id,
        name: advisor.name,
        personality: advisor.personality,
        currentPrompt: advisor.systemPrompt,
      }) as { success: boolean; prompt?: string; error?: string };
      if (!result.success || !result.prompt) {
        throw new Error(result.error || '优化失败');
      }
      await window.ipcRenderer.advisors.update({
        ...advisor,
        systemPrompt: result.prompt,
      });
      await refreshTeamAdvisor(advisor.id);
    } catch (error) {
      console.error('Failed to optimize advisor prompt:', error);
      void appAlert(`优化失败：${error instanceof Error ? error.message : '未知错误'}`);
    } finally {
      setIsTeamOptimizingPrompt(false);
    }
  }, [refreshTeamAdvisor]);

  const handlePromoteTeamMemberSkillCandidate = useCallback(async (advisor: Advisor) => {
    try {
      await window.ipcRenderer.advisors.promoteMemberSkillCandidate({
        advisorId: advisor.id,
        candidateVersion: advisor.memberSkillCandidateVersion,
      });
      await refreshTeamAdvisor(advisor.id);
    } catch (error) {
      console.error('Failed to promote member skill candidate:', error);
      setTestMsg('成员技能候选发布失败');
      setStatus('error');
    }
  }, [refreshTeamAdvisor]);

  const handleDiscardTeamMemberSkillCandidate = useCallback(async (advisor: Advisor) => {
    try {
      await window.ipcRenderer.advisors.discardMemberSkillCandidate({ advisorId: advisor.id });
      await refreshTeamAdvisor(advisor.id);
    } catch (error) {
      console.error('Failed to discard member skill candidate:', error);
      setTestMsg('成员技能候选丢弃失败');
      setStatus('error');
    }
  }, [refreshTeamAdvisor]);

  const handleRefreshTeamMemberSkill = useCallback(async (advisor: Advisor) => {
    try {
      const result = await window.ipcRenderer.advisors.distillMemberSkill({ advisorId: advisor.id }) as { success?: boolean; error?: string };
      if (result && result.success === false) {
        throw new Error(result.error || '成员技能蒸馏失败');
      }
      await refreshTeamAdvisor(advisor.id);
    } catch (error) {
      console.error('Failed to refresh member skill:', error);
      void appAlert(`成员技能蒸馏失败：${error instanceof Error ? error.message : '未知错误'}`);
    }
  }, [refreshTeamAdvisor]);

  const handleSaveTeamAdvisor = useCallback(async (
    data: Omit<Advisor, 'id' | 'createdAt' | 'knowledgeFiles'>,
    youtubeParams?: { url: string; count: number; channelId?: string },
    knowledgeFilePaths?: string[],
  ) => {
    let advisorId = editingTeamAdvisor?.id;
    if (editingTeamAdvisor) {
      await window.ipcRenderer.advisors.update({
        ...data,
        id: editingTeamAdvisor.id,
        redclawVisible: editingTeamAdvisor.redclawVisible !== false,
        redclawOrder: editingTeamAdvisor.redclawOrder,
      });
    } else {
      const createData: Record<string, unknown> = { ...data };
      if (youtubeParams?.url) {
        createData.youtubeChannel = {
          url: youtubeParams.url,
          channelId: youtubeParams.channelId || '',
        };
      }
      const result = await window.ipcRenderer.advisors.create(createData) as { success?: boolean; id?: string; error?: string };
      if (result?.success === false) {
        throw new Error(result.error || '创建成员失败');
      }
      advisorId = result?.id;
      if (advisorId && Array.isArray(knowledgeFilePaths) && knowledgeFilePaths.length > 0) {
        await window.ipcRenderer.advisors.uploadKnowledge({
          advisorId,
          filePaths: knowledgeFilePaths,
        });
      }
    }
    setEditingTeamAdvisor(null);
    setIsCreatingTeamAdvisor(false);
    await loadTeamAdvisors();
    if (advisorId) {
      await refreshTeamAdvisor(advisorId);
    }
    window.dispatchEvent(new Event('redclaw:team-settings-changed'));
  }, [editingTeamAdvisor, loadTeamAdvisors, refreshTeamAdvisor]);
  return {
    refreshTeamAdvisor,
    loadTeamAdvisors,
    teamAdvisors,
    isTeamAdvisorsLoading,
    teamAdvisorBusyId,
    draggingTeamAdvisorId,
    handleCreateTeamAdvisor,
    handleToggleTeamAdvisorVisible,
    handleOpenTeamAdvisorSettings,
    handleTeamAdvisorDragStart,
    handleTeamAdvisorDragOver,
    handleTeamAdvisorDragEnd,
    editingTeamAdvisor,
    isCreatingTeamAdvisor,
    handleSaveTeamAdvisor,
    setEditingTeamAdvisor,
    setIsCreatingTeamAdvisor,
    settingsTeamAdvisor,
    setSettingsTeamAdvisor,
    isTeamSystemPromptExpanded,
    setIsTeamSystemPromptExpanded,
    isTeamOptimizingPrompt,
    handleOptimizeTeamAdvisorPrompt,
    handleUploadTeamAdvisorKnowledge,
    handleDeleteTeamAdvisorKnowledge,
    handlePromoteTeamMemberSkillCandidate,
    handleDiscardTeamMemberSkillCandidate,
    handleRefreshTeamMemberSkill,
    handleDeleteTeamAdvisor,
  };
}
