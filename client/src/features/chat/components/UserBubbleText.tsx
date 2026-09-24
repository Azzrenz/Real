
import { useEffect, useState } from 'react';
import { Icon } from '../../../shared/ui/icons';
import { skillsApi } from '../../../services/domains/skills';

let skillNamesPromise: Promise<Set<string>> | null = null;
function loadSkillNames(): Promise<Set<string>> {
  if (!skillNamesPromise) {
    skillNamesPromise = skillsApi
      .list()
      .then((r) => new Set(r.skills.map((s) => s.name)))
      .catch(() => new Set<string>());
  }
  return skillNamesPromise;
}

export function UserBubbleText({ text }: { text: string }) {
  const [known, setKnown] = useState<Set<string> | null>(null);
  useEffect(() => {
    void loadSkillNames().then(setKnown);
  }, []);

  if (known) {
    const m = text.match(/^(?:\/[^\s/]+(?:\s|$))+/);
    if (m) {
      const names = m[0].trim().split(/\s+/).map((s) => s.replace(/^\//, ''));
      if (names.length > 0 && names.every((n) => known.has(n))) {
        const rest = text.slice(m[0].length);
        return (
          <>
            <span className="bubble-skill-row">
              {names.map((n) => (
                <span key={n} className="compose-skill-chip bubble-skill-chip">
                  <Icon name="flame" size={11} className="compose-skill-icon" />
                  <span className="compose-skill-name">{n}</span>
                </span>
              ))}
            </span>
            {rest && <span className="bubble-rest">{rest}</span>}
          </>
        );
      }
    }
  }
  return <>{text}</>;
}
