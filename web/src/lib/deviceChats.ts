import { useNodesStore } from '../stores/nodes';
import { useThreadsStore } from '../stores/threads';
import { useDevicesStore } from '../stores/devices';
import type { NodeInfo, Thread } from '../api/types';

/** Chats of every device as the drawer and All chats show them. */
export function useDeviceChats() {
  const threads = useThreadsStore((s) => s.threads);
  const nodes = useNodesStore((s) => s.nodes);
  const selectedId = useNodesStore((s) => s.selectedId);
  const lists = useDevicesStore((s) => s.lists);
  const access = useDevicesStore((s) => s.access);

  const self = nodes.find((n) => n.is_self);
  const selfId = self?.node_id ?? 'self';
  const openId = selectedId ?? selfId;
  const multi = nodes.length > 1;
  /**
   * Devices that let this one run agents are listed (this device always may). A device that has
   * not answered yet stays listed while it is offline (reconnecting); it goes when it answers
   * `agents: false` or is unpaired.
   */
  const allowed = (n: NodeInfo) => {
    if (n.is_self) return true;
    const a = access[n.node_id];
    return a ? a.agents : !n.online;
  };
  // Members in the network first (server order), then this device, offline devices last. An
  // offline device stays while its last known answer allowed agents.
  const order = [
    ...nodes.filter((n) => !n.is_self && n.online),
    ...nodes.filter((n) => n.is_self),
    ...nodes.filter((n) => !n.is_self && !n.online),
  ].filter(allowed);
  const reachable = (n: NodeInfo) => n.is_self || n.online;

  /** The open device's list is live; the others are the last fetched. */
  const listOf = (id: string): Thread[] =>
    id === openId && (threads.length || !lists[id]) ? threads : (lists[id] ?? []);

  /** Opens the device first when it is not the open one, so the chat is shown from there. */
  const goTo = (n: NodeInfo | undefined) => {
    if (n && n.node_id !== openId) useNodesStore.getState().select(n.is_self ? null : n.node_id);
  };

  return { nodes, self, selfId, openId, multi, order, allowed, reachable, listOf, goTo };
}
