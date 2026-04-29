# ATLAS Disaster Recovery Runbook

**Version:** 1.0  
**Owner:** Platform SRE  
**Last reviewed:** 2025-01-01  
**RPO target:** 1 hour  
**RTO target:** 4 hours

---

## 1. Scope

This runbook covers the procedures to restore ATLAS storage service following:

- Loss of a single storage node
- Loss of an entire availability zone (AZ)
- Accidental deletion of a volume or namespace
- Corruption of the metadata store
- Full data-centre / cloud-region outage

---

## 2. Prerequisites

- `atlasctl` v0.1+ installed on the recovery workstation
- Read access to the offsite backup bucket (`$ATLAS_BACKUP_BUCKET`)
- Write access to the target cluster's admin API
- A decryption key for the backup AES-256-GCM envelope (stored in Vault at `secret/atlas/backup-key`)

---

## 3. Node failure (single node)

**Symptoms:** Replication factor alerts fire; `atlasctl status` shows one node `DEGRADED`.

```
atlasctl node list
atlasctl node drain <node-id>     # drain in-flight writes
atlasctl node remove <node-id>    # remove from cluster membership
atlasctl node add <replacement>   # provision replacement
atlasctl node rebalance           # re-distribute shards
```

**Expected recovery time:** 30–60 min (depends on data volume and network throughput).

---

## 4. AZ failure

**Symptoms:** 33 %+ of nodes offline; cross-AZ replication lag > 5 min.

1. Confirm the AZ is unavailable via the cloud-provider console.
2. Promote the replica leader in a surviving AZ:

```
atlasctl cluster failover --az <failed-az> --promote-leader
```

3. Update DNS / load-balancer to route only to surviving AZs.
4. When the failed AZ recovers, re-add nodes and rebalance:

```
atlasctl node add --az <recovered-az> <node-id> ...
atlasctl node rebalance
```

**Expected recovery time:** 1–2 hours.

---

## 5. Accidental deletion of a volume

```
# List recent snapshots for the deleted volume
atlasctl backup list --volume <vol-id>

# Restore the most recent snapshot
atlasctl backup restore \
  --bundle s3://$ATLAS_BACKUP_BUCKET/<vol-id>/latest.atlas-bundle \
  --target-volume <vol-id>-restored
```

Verify the restored volume:

```
atlasctl fs stat --volume <vol-id>-restored /
atlasctl fs list --volume <vol-id>-restored / --recursive | head -50
```

**Expected recovery time:** 15–45 min.

---

## 6. Metadata-store corruption (sled / FoundationDB)

**Symptoms:** `atlas-storage` fails to start; metadata logs show `corruption` or CRC errors.

```
# Stop all storage nodes to prevent further writes
atlasctl cluster stop --all

# Restore metadata from the last clean checkpoint
atlasctl backup restore-meta \
  --snapshot s3://$ATLAS_BACKUP_BUCKET/meta/latest-meta.tar.gz \
  --dest /var/lib/atlas/meta

# Re-start storage nodes
atlasctl cluster start --all
atlasctl cluster health --wait
```

**Expected recovery time:** 45–90 min.

---

## 7. Full region outage (cross-region DR)

1. Declare a regional incident in PagerDuty and notify stakeholders.
2. Switch traffic to the DR region:

```
# Update global load-balancer
atlasctl dr failover --primary-region <failed> --dr-region <target>
```

3. Restore from the most recent cross-region replica bundle:

```
atlasctl backup restore \
  --bundle s3://$ATLAS_DR_BUCKET/<vol-id>/latest.atlas-bundle \
  --target-cluster $ATLAS_DR_CLUSTER \
  --volume <vol-id>
```

4. Validate data integrity and resume service.
5. File a post-mortem within 5 business days.

**Expected recovery time:** 2–4 hours.

---

## 8. Post-recovery checklist

- [ ] All nodes report `HEALTHY` in `atlasctl node list`
- [ ] Replication lag < 30 s on all volumes
- [ ] No pending chunk-repair jobs
- [ ] Backup schedule re-enabled and next run confirmed
- [ ] Incident ticket updated with timeline and root cause
- [ ] Post-mortem scheduled

---

## 9. Contacts

| Role | Contact |
|---|---|
| On-call SRE | PagerDuty escalation policy `atlas-prod` |
| Data Engineering lead | data-eng@example.com |
| Security officer | security@example.com |
