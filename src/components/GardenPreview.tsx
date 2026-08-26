import { BookOpenText, MessageCircleMore, Sparkles } from "lucide-react";
import type { Garden, SkillProgress, SkillStrand } from "../types";

const strandMeta: Record<
  SkillStrand,
  { label: string; className: string; Icon: typeof Sparkles }
> = {
  vocabulary: { label: "Words", className: "vocabulary", Icon: Sparkles },
  grammar: { label: "Patterns", className: "grammar", Icon: BookOpenText },
  fluency: { label: "Flow", className: "fluency", Icon: MessageCircleMore },
};

export function GardenPreview({ garden, compact = false }: { garden: Garden; compact?: boolean }) {
  return (
    <div className={`garden-preview ${compact ? "garden-preview--compact" : ""}`}>
      <div className="garden-sky" aria-hidden="true">
        <span className="garden-sun" />
        <span className="garden-cloud garden-cloud--one" />
        <span className="garden-cloud garden-cloud--two" />
      </div>
      <div className="garden-field" aria-hidden="true" />
      <div className="garden-plots">
        {garden.skills.map((skill) => (
          <SkillPlant key={skill.id} skill={skill} compact={compact} />
        ))}
      </div>
      {!compact && (
        <div className="garden-caption">
          <span>{garden.level_name}</span>
          <small>{garden.total_conversations} conversations</small>
        </div>
      )}
    </div>
  );
}

function SkillPlant({ skill, compact }: { skill: SkillProgress; compact: boolean }) {
  const meta = strandMeta[skill.strand];
  const plant = plantAsset(skill);
  return (
    <div className={`skill-plant skill-plant--${meta.className}`}>
      <div className="plot-visual">
        <img className="plot-tile" src="/assets/garden/tiles-turf.svg" alt="" />
        {plant ? (
          <img className="plant-art" src={plant} alt="" />
        ) : (
          <span className="plant-seed" aria-hidden="true" />
        )}
      </div>
      {!compact && (
        <div className="skill-label">
          <meta.Icon size={15} aria-hidden="true" />
          <div>
            <strong>{meta.label}</strong>
            <span>{skill.stage_label}</span>
          </div>
        </div>
      )}
    </div>
  );
}

function plantAsset(skill: SkillProgress): string | null {
  if (skill.stage === 0) return null;
  const prefix = skill.strand === "vocabulary" ? "vocab" : skill.strand;
  const stage = skill.stage === 1 ? "seedling" : skill.stage === 2 ? "young" : "bloom";
  return `/assets/garden/${prefix}-${stage}.svg`;
}
