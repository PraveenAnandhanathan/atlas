# ATLAS Game-Day Playbook

**Version:** 1.0  
**Owner:** Platform SRE  
**Cadence:** Quarterly  
**Last run:** 2025-01-01

---

## 1. Purpose

Game-day exercises validate that:

1. DR runbooks are accurate and actionable.
2. On-call engineers can execute recovery steps under pressure.
3. Monitoring, alerting, and dashboards surface failures quickly.
4. RTO / RPO targets are achievable.

---

## 2. Scenarios

### Scenario A — Node crash (30 min)

**Inject:**

```
atlas-chaos run node-crash
```

**Expected outcomes:**
- PagerDuty alert fires within 2 min.
- `atlasctl cluster health` shows node `DEGRADED`.
- Automatic leader re-election completes within 30 s.
- No client-visible errors after 60 s.

**Pass criteria:** Service restored without manual intervention in < 5 min.

---

### Scenario B — Network partition (45 min)

**Inject:**

```
atlas-chaos run network-partition
```

**Expected outcomes:**
- Writes to isolated node return `UNAVAILABLE`.
- Reads from quorum remain available.
- Partition heals automatically when fault is removed.
- No data loss (WAL flush before partition).

**Pass criteria:** 0 bytes lost; no zombie writes accepted.

---

### Scenario C — Disk full (30 min)

**Inject:**

```
atlas-chaos run disk-full
```

**Expected outcomes:**
- `atlas-storage` returns `ENOSPC` after quota enforcement.
- Operator receives alert with affected node and volume.
- After clearing disk space, service resumes automatically.

**Pass criteria:** No data corruption; alert < 5 min.

---

### Scenario D — Backup restore drill (60 min)

**Inject:** Take an ad-hoc snapshot then truncate the target volume.

```
atlasctl backup export --out /tmp/game-day.atlas-bundle
atlasctl volume truncate --volume test-vol --confirm
atlasctl backup restore --bundle /tmp/game-day.atlas-bundle --target-volume test-vol
```

**Expected outcomes:**
- Restore completes within RTO target.
- BLAKE3 footer verification passes.
- Spot-check of 10 random files matches pre-truncation hashes.

**Pass criteria:** 100 % file-content match; RTO < 4 h.

---

### Scenario E — Chaos suite (full nightly run, 2 h)

```
atlas-chaos suite
```

Runs all built-in scenarios sequentially, capturing a `ChaosReport` for each. Review the report JSON for invariant violations before ending the session.

**Pass criteria:** 0 invariant violations.

---

## 3. Roles

| Role | Responsibility |
|---|---|
| Incident commander | Calls play start/stop; tracks time |
| Primary SRE | Executes runbook steps |
| Observer | Documents deviations; files follow-up tickets |
| Comms lead | Updates status page and stakeholders |

---

## 4. Schedule

| Time | Activity |
|---|---|
| T+0:00 | Briefing; confirm pre-conditions; enable chaos module |
| T+0:15 | Scenario A |
| T+0:45 | Scenario B |
| T+1:30 | Scenario C |
| T+2:00 | Scenario D |
| T+3:00 | Debrief; capture action items |

---

## 5. Post-game-day actions

1. File tickets for every runbook step that needed improvisation.
2. Update DR runbook with corrections within 48 h.
3. If any RTO / RPO target was missed, schedule a follow-up in 4 weeks.
4. Archive the `ChaosReport` JSON in `gs://$ATLAS_AUDIT_BUCKET/game-day/<date>/`.

---

## 6. Go / No-go criteria

Abort the game day and restore production if any of the following occur:

- Real customer traffic is impacted unexpectedly.
- A fault injection cannot be cleanly removed.
- Data loss is detected outside the targeted test volume.
