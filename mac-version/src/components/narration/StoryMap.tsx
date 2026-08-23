import { useRef, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Map, X } from "lucide-react";
import { useTranslation } from "react-i18next";

interface NarrationMessage {
  id: string;
  role: "narrator" | "user";
  content: string;
  created_at: string;
}

interface StoryMapProps {
  open: boolean;
  onClose: () => void;
  messages: NarrationMessage[];
  sceneImages: Record<number, string>;
  /** Called with message index to scroll the main view to that position. */
  onJumpToMessage: (index: number) => void;
  onForkFromMessage?: (messageId: string) => void;
  embedded?: boolean;
}

/**
 * StoryMap — scene index overlay showing each narrative beat as a card.
 *
 * Renders user actions as entry cards and narrator responses as scene
 * cards. Generated scene images appear on their parent narrator card.
 * Clicking a card scrolls the NarrationScreen to that message.
 *
 * Pure frontend — no backend data needed. Uses the message list and
 * scene images already held in NarrationScreen state.
 */
export function StoryMap({ open, onClose, messages, sceneImages, onJumpToMessage, onForkFromMessage, embedded = false }: StoryMapProps) {
  const { t } = useTranslation();
  const panelRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");

  // Build scene entries: each user-action + following narrator response = one "scene"
  const scenes = (() => {
    const result: { userIdx: number; narratorIdx: number | null; action: string; timestamp: string }[] = [];
    for (let i = 0; i < messages.length; i++) {
      if (messages[i].role === "user") {
        const narratorIdx = i + 1 < messages.length && messages[i + 1].role === "narrator" ? i + 1 : null;
        result.push({
          userIdx: i,
          narratorIdx,
          action: messages[i].content.slice(0, 60),
          timestamp: messages[i].created_at,
        });
      }
    }
    return result;
  })();

  return (
    <AnimatePresence>
      {open && (
        <>
          {/* Backdrop */}
          {!embedded && <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            className="fixed inset-0 z-40 bg-black/30"
            onClick={onClose}
          />}

          {/* Panel — slides from right */}
          <motion.div
            ref={panelRef}
            initial={{ opacity: 0, x: 40 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 40 }}
            transition={{ duration: 0.2, ease: "easeOut" }}
            className={embedded
              ? "w-full h-full overflow-y-auto"
              : "fixed right-0 top-10 bottom-0 z-50 w-[320px] overflow-y-auto surface-elevated border-l border-[var(--color-separator)]/30 shadow-2xl"}
            onClick={(e) => e.stopPropagation()}
          >
            {/* Header */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--color-separator)]/20 sticky top-0 bg-[var(--color-bg-elevated)]/95 backdrop-blur-xl z-10">
              <div className="flex items-center gap-2">
                <Map className="w-3.5 h-3.5 text-[var(--color-accent)]" />
                <h3 className="text-[11px] font-display tracking-[0.04em] text-[var(--color-label-primary)]">
                  {t("ui.story-map", "Story Map")}
                </h3>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-[var(--color-label-tertiary)]">
                  {scenes.length} scenes
                </span>
                <button
                  onClick={onClose}
                  className="p-1 rounded-md text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
                  aria-label={t("ui.close-story-map", "Close story map")}
                >
                  <X className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>

            {/* Scene list */}
            <div className="p-3 space-y-3">
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("ui.find-a-scene", "Find a scene...")} aria-label={t("ui.search-story-scenes", "Search story scenes")} className="w-full rounded-lg bg-[var(--color-fill-tertiary)] px-2.5 py-2 text-[11px] text-[var(--color-label-primary)] outline-none" />
              {scenes.length === 0 ? (
                <div className="text-center py-12">
                  <Map className="w-6 h-6 mx-auto mb-2 text-[var(--color-label-quaternary)]" />
                  <p className="text-[11px] text-[var(--color-label-tertiary)]">
                    {t("ui.no-scenes-yet-your-story-map-will-appear-here-as", "No scenes yet. Your story map will appear here as you play.")}
                  </p>
                </div>
              ) : (
                scenes.filter((scene) => !query || scene.action.toLowerCase().includes(query.toLowerCase())).map((scene, sceneIdx) => (
                  <div
                    key={scene.userIdx}
                    className="w-full text-left rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 hover:border-[var(--color-accent)]/30 hover:bg-[var(--color-accent-soft)] transition-all overflow-hidden group"
                  >
                    {sceneIdx % 5 === 0 && <p className="px-2.5 pt-2 text-[9px] font-medium uppercase tracking-wider text-[var(--color-label-quaternary)]">Chapter {Math.floor(sceneIdx / 5) + 1}</p>}
                    {/* Scene image (if available) */}
                    {scene.narratorIdx !== null && sceneImages[scene.narratorIdx] && (
                      <div className="aspect-[16/9] overflow-hidden bg-[var(--color-fill-tertiary)]">
                        <img
                          src={sceneImages[scene.narratorIdx]}
                          alt={`Scene ${sceneIdx + 1}`}
                          className="w-full h-full object-cover group-hover:scale-[1.02] transition-transform duration-300"
                          loading="lazy"
                        />
                      </div>
                    )}

                    {/* Scene info */}
                    <button onClick={() => { onJumpToMessage(scene.userIdx); onClose(); }} className="w-full p-2.5 text-left">
                      <div className="flex items-center gap-2 mb-1">
                        <span className="text-[10px] font-display tracking-[0.04em] text-[var(--color-accent)]">
                          Scene {sceneIdx + 1}
                        </span>
                        <span className="text-[9px] text-[var(--color-label-quaternary)]">
                          {new Date(scene.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                        </span>
                      </div>
                      <p className="text-[11px] text-[var(--color-label-secondary)] leading-relaxed line-clamp-2 font-serif italic">
                        {scene.action}
                        {scene.action.length >= 60 ? "…" : ""}
                      </p>
                    </button>
                    {onForkFromMessage && <button onClick={() => onForkFromMessage(messages[scene.userIdx].id)} className="mx-2.5 mb-2 text-[10px] text-[var(--color-accent)] hover:underline">{t("ui.fork-from-here", "Fork from here")}</button>}
                  </div>
                ))
              )}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
