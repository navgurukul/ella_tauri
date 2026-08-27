import { useEffect, useRef, useState } from "react";
import { EllaMascot } from "./EllaMascot";
import { levelInfo, units } from "../lib/presentation";
import type { AppSnapshot, SkillEvidence, UnitNode } from "../types";

/** The garden is a fixed-coordinate scene; it scales to fit the window. */
const STAGE_WIDTH = 1110;
const STAGE_HEIGHT = 600;

export function GardenScreen({
  snapshot,
  reveal,
  onTalk,
}: {
  snapshot: AppSnapshot;
  reveal?: SkillEvidence | null;
  /** Starts the topic that practises this plant's strand; null means "Ella picks". */
  onTalk: (topicId: string | null) => void;
}) {
  const level = levelInfo(snapshot.garden, snapshot.learner?.level_name);
  const nodes = units(snapshot.garden, snapshot.topics);
  const frame = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(1);

  useEffect(() => {
    const element = frame.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setScale(Math.min(1, width / STAGE_WIDTH, height / STAGE_HEIGHT));
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  return (
    <div className="screen screen--garden" data-screen="garden">
      <header className="garden-head">
        <div className="garden-head__title">
          <h1 className="display display--md">Your garden</h1>
          <span className="pill pill--violet">Level {level.code}</span>
        </div>
        <div className="garden-head__progress">
          <div className="meter meter--wide">
            <span style={{ width: `${Math.round(level.ratio * 100)}%` }} />
          </div>
          <strong>
            {level.skillsDone} / {level.skillsTotal} skills
          </strong>
        </div>
      </header>

      {reveal && (
        <p className="reveal" role="status">
          <span className="reveal__dot" aria-hidden="true" />
          Your “{reveal.skill_label}” plant just grew. Nice talking!
        </p>
      )}

      <div className="garden-frame" ref={frame}>
        <div
          className="garden-stage"
          style={{
            width: STAGE_WIDTH,
            height: STAGE_HEIGHT,
            transform: `scale(${scale})`,
          }}
        >
          <Scenery />
          <svg className="garden-path" width={STAGE_WIDTH} height={STAGE_HEIGHT} viewBox="0 0 1110 600" aria-hidden="true">
            <path d="M -60 170 L 870 170 C 1072 170 1072 440 870 440 L 380 440" />
            <path className="garden-path__dashed" d="M 340 440 L 210 440" />
          </svg>
          {nodes.map((unit) => (
            <UnitMarker key={unit.num} unit={unit} onTalk={onTalk} />
          ))}
        </div>
      </div>

      <EllaMascot className="ella--corner-garden" scale={0.61} rotate={4} />
    </div>
  );
}

function UnitMarker({
  unit,
  onTalk,
}: {
  unit: UnitNode;
  onTalk: (topicId: string | null) => void;
}) {
  const locked = unit.state === "locked";
  return (
    <div className="unit" style={{ left: unit.x - 75, top: unit.y - 50 }}>
      <button
        className={`unit__node ${unit.current ? "is-current" : ""}`}
        disabled={locked}
        onClick={() => onTalk(unit.topicId)}
        aria-label={`Unit ${unit.num}: ${unit.name}${locked ? " (locked)" : ""}`}
      >
        <span className="unit__ring" aria-hidden="true" />
        <span className="unit__tint" style={{ background: unit.tint }} aria-hidden="true">
          <Plant state={unit.state} />
        </span>
        {unit.current && (
          <svg className="unit__arc" width="116" height="116" viewBox="0 0 116 116" aria-hidden="true">
            <circle cx="58" cy="58" r="52" strokeDasharray="218 327" transform="rotate(-90 58 58)" />
          </svg>
        )}
        {unit.done && (
          <span className="unit__check" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path d="M20 6L9 17L4 12" />
            </svg>
          </span>
        )}
      </button>
      <span className="mono unit__num">UNIT {unit.num}</span>
      <span className="unit__name">{unit.name}</span>
    </div>
  );
}

function Plant({ state }: { state: UnitNode["state"] }) {
  if (state === "bloom") {
    return (
      <span className="plant plant--bloom">
        <i className="plant__stem" />
        <i className="plant__petal plant__petal--w" />
        <i className="plant__petal plant__petal--e" />
        <i className="plant__petal plant__petal--n" />
        <i className="plant__petal plant__petal--s" />
        <i className="plant__heart" />
        <i className="plant__soil" />
      </span>
    );
  }
  if (state === "young") {
    return (
      <span className="plant plant--young">
        <i className="plant__stem" />
        <i className="plant__leaf plant__leaf--left" />
        <i className="plant__leaf plant__leaf--right" />
        <i className="plant__soil" />
      </span>
    );
  }
  if (state === "seedling") {
    return (
      <span className="plant plant--seedling">
        <i className="plant__stem" />
        <i className="plant__leaf plant__leaf--left" />
        <i className="plant__soil" />
      </span>
    );
  }
  if (state === "bare") {
    return (
      <span className="plant plant--bare">
        <i className="plant__soil" />
      </span>
    );
  }
  return (
    <span className="plant plant--locked">
      <i className="plant__shackle" />
      <i className="plant__lock" />
    </span>
  );
}

/** Twigs, pebbles and a shed, all in faded soil brown, exactly as drawn. */
function Scenery() {
  return (
    <div className="scenery" aria-hidden="true">
      <div className="scenery__shed">
        <span className="scenery__twigs">
          <i />
          <i />
          <i />
        </span>
        <span className="scenery__shed-roof" />
        <span className="scenery__shed-body" />
      </div>
      <span className="scenery__twigs scenery__twigs--left">
        <i />
        <i />
        <i />
      </span>
      <span className="scenery__twigs scenery__twigs--bottom">
        <i />
        <i />
        <i />
      </span>
      <span className="scenery__pebble scenery__pebble--a" />
      <span className="scenery__pebble scenery__pebble--b" />
      <span className="scenery__pebble scenery__pebble--c" />
    </div>
  );
}
