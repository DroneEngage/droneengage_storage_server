const logger = require('./logger');
const database = require('./database');
const serverConfig = require('../js_serverConfig.js');

// Message type constants (matching AndruavMessageTypes)
const CONST_TYPE_AndruavSystem_LoadTasks = 9001;
const CONST_TYPE_AndruavSystem_SaveTasks = 9002;
const CONST_TYPE_AndruavSystem_DeleteTasks = 9003;
const CONST_TYPE_AndruavSystem_DisableTasks = 9004;
const CONST_TYPE_AndruavSystem_UnitOnline = 9009; // Custom message for unit online event (aligned with shared AndruavMessageTypes enum)
const CONST_TYPE_AndruavSystem_LoadMission = 9010; // Load mission from storage
const CONST_TYPE_AndruavSystem_SaveMission = 9011; // Save mission to storage
const CONST_TYPE_AndruavSystem_DeleteMission = 9012; // Delete mission from storage
const CONST_TYPE_AndruavSystem_LoadNews = 9015; // Load news (account + global) from storage
const CONST_TYPE_AndruavSystem_SaveNews = 9016; // Save news to storage
const CONST_TYPE_AndruavSystem_DeleteNews = 9017; // Delete/disable news from storage
const CONST_TYPE_AndruavSystem_NewsPush = 9018; // Unsolicited push: storage -> comm servers

// Allowed mission kinds (P1F-01): v1 plans, v2 operations, compiled packages
// (P4A-09 deploy archive), flight journals (Phase 6) and the static op
// story graph (TASK-SG-02 — one row per op_hash).
const MISSION_KINDS = new Set(['de_plan', 'de_operation', 'de_package', 'de_journal', 'de_opgraph']);
// meta is a small JSON descriptor stored beside the mission (2 KB cap).
const CONST_MISSION_META_MAX_BYTES = 2048;
// Default cap on the serialized `data` column when limits.max_mission_bytes
// is absent from server.config.
const CONST_MISSION_DATA_MAX_BYTES_DEFAULT = 1024 * 1024;

class MessageHandlers {
  constructor(db, wsServer) {
    this.db = db;
    this.wsServer = wsServer;
  }

  /**
   * Extract Andruav payload from the ms field (falls back to the message itself for tests).
   */
  getPayload(message) {
    return (message && typeof message.ms === 'object' && message.ms !== null) ? message.ms : message;
  }

  /**
   * Echoes the request id (rid) from the incoming message, if present, so callers
   * (e.g. DBProxyClient on the comm server) can correlate concurrent requests/responses.
   */
  buildResponseEnvelope(message, mt, ms, success, error) {
    const envelope = { mt, ms, success, timestamp: Date.now() };
    if (message && message.rid != null) envelope.rid = message.rid;
    if (error !== undefined) envelope.error = error;
    return envelope;
  }

  /**
   * Handle LoadTasks message (9001)
   */
  async handleLoadTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.debug(`LoadTasks request for unit ${unitId} from comm server ${connection.commServerId}`);

      // Get tasks from database
      const tasks = this.db.loadTasks(unitId);

      // Log access
      this.db.logAccess(unitId, 'load', 'task', null, connection.commServerId);

      // Send response - use 'ms' field for response data (matching existing system)
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadTasks, {
        s: 'OK',
        tasks: tasks,
        unitId: unitId
      }, true));

      logger.debug(`Loaded ${tasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error loading tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle SaveTasks message (9002)
   */
  async handleSaveTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, tasks } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`SaveTasks request for unit ${unitId} with ${tasks.length} tasks`);

      // Ensure unit exists
      this.db.upsertUnit(unitId, null, connection.commServerId);

      // Save each task
      const savedTasks = [];
      for (const task of tasks) {
        const { taskId, name, data } = task;
        this.db.saveTask(taskId, unitId, name, data);
        savedTasks.push(taskId);
      }

      // Log access
      this.db.logAccess(unitId, 'save', 'task', savedTasks.join(','), connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveTasks, {
        s: 'OK:save',
        savedTasks: savedTasks,
        unitId: unitId
      }, true));

      logger.info(`Saved ${savedTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error saving tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle DeleteTasks message (9003)
   */
  async handleDeleteTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, taskIds } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`DeleteTasks request for unit ${unitId} with ${taskIds.length} tasks`);

      // Delete each task
      const deletedTasks = [];
      for (const taskId of taskIds) {
        this.db.deleteTask(taskId);
        deletedTasks.push(taskId);
      }

      // Log access
      this.db.logAccess(unitId, 'delete', 'task', deletedTasks.join(','), connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteTasks, {
        s: 'OK:del',
        deletedTasks: deletedTasks,
        unitId: unitId
      }, true));

      logger.info(`Deleted ${deletedTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error deleting tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle DisableTasks message (9004)
   */
  async handleDisableTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, taskIds } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`DisableTasks request for unit ${unitId} with ${taskIds.length} tasks`);

      // Disable each task
      const disabledTasks = [];
      for (const taskId of taskIds) {
        this.db.disableTask(taskId);
        disabledTasks.push(taskId);
      }

      // Log access
      this.db.logAccess(unitId, 'disable', 'task', disabledTasks.join(','), connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DisableTasks, {
        s: 'OK:disable',
        disabledTasks: disabledTasks,
        unitId: unitId
      }, true));

      logger.info(`Disabled ${disabledTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error disabling tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DisableTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle UnitOnline message (9009) - Trigger offline queue processing
   */
  async handleUnitOnline(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`Unit online event for unit ${unitId} from comm server ${connection.commServerId}`);

      // Process offline queue for this unit
      const OfflineQueue = require('./offlineQueue');
      const offlineQueue = new OfflineQueue(this.db, this.wsServer);
      
      const result = await offlineQueue.processQueueForUnit(unitId, connectionId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_UnitOnline, {
        s: 'OK',
        unitId: unitId,
        processed: result.processed,
        failed: result.failed
      }, true));

      logger.info(`Unit online processed for unit ${unitId}: ${result.processed} messages delivered, ${result.failed} failed`);
    } catch (error) {
      logger.error(`Error processing unit online for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_UnitOnline, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle LoadMission message (9010)
   *
   * Three request forms:
   *   {list:true, kind?, unitId?, before?} — P1F-01 metadata-only listing
   *     (no `data` column), account-scoped, newest first, max 200.
   *   {missionId} — one full row including `data` (account-scoped).
   *   {unitId}  — legacy form: all full rows for the unit (unchanged).
   *
   * `rq` is an opaque client request id echoed back inside `ms` so callers
   * can match replies to requests (P1F-02).
   */
  async handleLoadMission(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, missionId, accountId, list, kind, before, rq } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      if (list === true) {
        const missions = this.db.listMissionsMeta(accountId, { kind, unitId, before });
        this.db.logAccess(unitId || null, 'load', 'mission', null, connection.commServerId);
        this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadMission, {
          s: 'OK',
          list: true,
          missions: missions,
          unitId: unitId || null,
          rq: rq
        }, true));
        logger.info(`Listed ${missions.length} missions for account ${accountId}`);
        return;
      }

      logger.info(`LoadMission request for unit ${unitId}${missionId ? ', mission ' + missionId : ''} from comm server ${connection.commServerId}`);

      let mission;
      if (missionId) {
        // Load specific mission
        mission = this.db.getMission(missionId, accountId);
      } else {
        // Load all missions for unit (scoped by account)
        const missions = this.db.loadMissions(unitId, accountId);
        mission = missions;
      }

      // Log access
      this.db.logAccess(unitId, 'load', 'mission', missionId || null, connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadMission, {
        s: 'OK',
        mission: mission,
        unitId: unitId,
        rq: rq
      }, true));

      logger.info(`Loaded mission for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error loading mission for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadMission, {
        s: 'ERROR:' + error.message,
        unitId: unitId,
        rq: rq
      }, false, error.message));
    }
  }

  /**
   * Handle SaveMission message (9011)
   *
   * P1F-01 additions over the legacy form:
   *  - `kind` (default 'de_plan') validated against MISSION_KINDS.
   *  - `meta` small JSON descriptor (≤ 2 KB) stored beside the row.
   *  - `missionId` may be omitted — a UUID v4 is generated server-side and
   *    returned in the reply.
   *  - A client-supplied id that exists under another account is rejected;
   *    it is never overwritten.
   *  - `limits.max_mission_bytes` (default 1 MB) caps the serialized data.
   *  - `limits.max_missions_per_kind.<kind>` optional extra quota on inserts.
   *  - `rq` echoed in `ms` for request/reply correlation.
   */
  async handleSaveMission(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, accountId, name, data, kind, meta, rq } = payload;
    const connection = this.wsServer.getConnection(connectionId);
    let c_missionId = payload.missionId;

    try {
      const c_kind = (kind === null || kind === undefined || kind === '') ? 'de_plan' : kind;
      if (!MISSION_KINDS.has(c_kind)) {
        this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
          s: `ERROR:unknown mission kind '${c_kind}'`,
          missionId: c_missionId,
          unitId: unitId,
          rq: rq
        }, false, 'unknown mission kind'));
        return;
      }

      // Server-generated id when the caller does not supply one.
      if (c_missionId === null || c_missionId === undefined || c_missionId === '') {
        c_missionId = require('crypto').randomUUID();
      }

      logger.info(`SaveMission request for unit ${unitId}, mission ${c_missionId} (kind ${c_kind})`);

      // Ensure unit exists
      this.db.upsertUnit(unitId, null, connection.commServerId);

      const limits = serverConfig.m_configuration.limits || {};

      // Size cap on the serialized payload.
      const maxBytes = limits.max_mission_bytes ?? CONST_MISSION_DATA_MAX_BYTES_DEFAULT;
      const dataBytes = Buffer.byteLength(JSON.stringify(data ?? null));
      if (maxBytes > 0 && dataBytes > maxBytes) {
        logger.warn(`SaveMission rejected: mission ${c_missionId} is ${dataBytes} bytes (limit ${maxBytes})`);
        this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
          s: `ERROR:mission too large (${dataBytes} bytes > ${maxBytes})`,
          missionId: c_missionId,
          unitId: unitId,
          rq: rq
        }, false, 'mission too large'));
        return;
      }

      // meta is a small descriptor — cap it.
      if (meta !== null && meta !== undefined &&
          Buffer.byteLength(JSON.stringify(meta)) > CONST_MISSION_META_MAX_BYTES) {
        this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
          s: `ERROR:mission meta too large (> ${CONST_MISSION_META_MAX_BYTES} bytes)`,
          missionId: c_missionId,
          unitId: unitId,
          rq: rq
        }, false, 'mission meta too large'));
        return;
      }

      // Ownership: an id that exists under another account is rejected
      // outright — the old ON CONFLICT upsert would have overwritten it.
      const ownerAccount = this.db.getMissionAccountId(c_missionId);
      if (ownerAccount !== null && ownerAccount !== accountId) {
        logger.warn(`SaveMission rejected: id ${c_missionId} belongs to another account (caller ${accountId})`);
        this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
          s: 'ERROR:mission id belongs to another account',
          missionId: c_missionId,
          unitId: unitId,
          rq: rq
        }, false, 'mission id belongs to another account'));
        return;
      }

      // Quotas gate NEW inserts only, not updates of an existing mission.
      if (ownerAccount === null) {
        const perAccount = limits.max_missions_per_account ?? 0;
        const perUnit = limits.max_missions_per_unit ?? 0;

        if (perAccount > 0 && this.db.countMissionsByAccount(accountId) >= perAccount) {
          const count = this.db.countMissionsByAccount(accountId);
          logger.warn(`SaveMission rejected: account ${accountId} at ${count}/${perAccount}`);
          this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
            s: `ERROR:mission quota exceeded (${count}/${perAccount} per account)`,
            missionId: c_missionId,
            unitId: unitId,
            rq: rq
          }, false, 'mission quota exceeded'));
          return;
        }
        if (perUnit > 0 && this.db.countMissionsByUnit(unitId, accountId) >= perUnit) {
          const count = this.db.countMissionsByUnit(unitId, accountId);
          logger.warn(`SaveMission rejected: account ${accountId} unit ${unitId} at ${count}/${perUnit}`);
          this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
            s: `ERROR:mission quota exceeded (${count}/${perUnit} per unit)`,
            missionId: c_missionId,
            unitId: unitId,
            rq: rq
          }, false, 'mission quota exceeded'));
          return;
        }
        const perKind = (limits.max_missions_per_kind && limits.max_missions_per_kind[c_kind]) ?? 0;
        if (perKind > 0 && this.db.countMissionsByKind(accountId, c_kind) >= perKind) {
          const count = this.db.countMissionsByKind(accountId, c_kind);
          logger.warn(`SaveMission rejected: account ${accountId} kind ${c_kind} at ${count}/${perKind}`);
          this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
            s: `ERROR:mission quota exceeded (${count}/${perKind} per kind ${c_kind})`,
            missionId: c_missionId,
            unitId: unitId,
            rq: rq
          }, false, 'mission quota exceeded'));
          return;
        }
      }

      // Save mission
      this.db.saveMission(c_missionId, unitId, accountId, name, data, c_kind, meta);

      // Log access
      this.db.logAccess(unitId, 'save', 'mission', c_missionId, connection.commServerId);

      // Send response — missionId + version let the caller track the row.
      const saved = this.db.getMission(c_missionId, accountId);
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
        s: 'OK:save',
        missionId: c_missionId,
        unitId: unitId,
        kind: c_kind,
        version: saved ? saved.version : 1,
        rq: rq
      }, true));

      logger.info(`Saved mission ${c_missionId} for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error saving mission for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveMission, {
        s: 'ERROR:' + error.message,
        missionId: c_missionId,
        unitId: unitId,
        rq: rq
      }, false, error.message));
    }
  }

  /**
   * Handle DeleteMission message (9012)
   */
  async handleDeleteMission(connectionId, message) {
    const payload = this.getPayload(message);
    const { missionId, accountId } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`DeleteMission request for mission ${missionId}`);

      const result = this.db.deleteMission(missionId, accountId);

      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteMission, {
        s: 'OK:delete',
        missionId: missionId,
        deleted: result.changes
      }, true));

      logger.info(`Deleted mission ${missionId} (changes: ${result.changes})`);
    } catch (error) {
      logger.error(`Error deleting mission ${missionId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteMission, {
        s: 'ERROR:' + error.message,
        missionId: missionId
      }, false, error.message));
    }
  }

  /**
   * Handle LoadNews message (9015)
   * Returns active (non-disabled, non-expired) global news + the caller's account news.
   */
  async handleLoadNews(connectionId, message) {
    const payload = this.getPayload(message);
    const { accountId } = payload;

    try {
      logger.debug(`LoadNews request for account ${accountId}`);

      const news = this.db.loadNews(accountId);

      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadNews, {
        s: 'OK',
        news: news
      }, true));

      logger.debug(`Loaded ${news.length} news items for account ${accountId}`);
    } catch (error) {
      logger.error(`Error loading news for account ${accountId}: ${error.message}`);

      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadNews, {
        s: 'ERROR:' + error.message
      }, false, error.message));
    }
  }

  /**
   * Handle SaveNews message (9016)
   */
  async handleSaveNews(connectionId, message) {
    const payload = this.getPayload(message);
    const { newsId, scope, accountId, title, body, priority, authorId, expiresAt } = payload;

    try {
      const id = newsId || require('crypto').randomUUID();
      logger.info(`SaveNews request ${id} (scope=${scope}, account=${accountId || 'null'})`);

      // Per-account quota: only gate NEW account-scoped inserts, not updates
      // of an existing newsId (preserves ON CONFLICT(id) DO UPDATE semantics).
      // Global news (scope='global', admin-dashboard-only) is exempt.
      const existing = this.db.getNews(id);
      if (!existing && scope === 'account' && accountId) {
        const limit = serverConfig.m_configuration.limits?.max_news_per_account ?? 0;
        if (limit > 0) {
          const count = this.db.countNewsByAccount(accountId);
          if (count >= limit) {
            logger.warn(`SaveNews rejected: account ${accountId} at ${count}/${limit}`);
            this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveNews, {
              s: `ERROR:news quota exceeded (${count}/${limit})`,
              newsId: id
            }, false, 'news quota exceeded'));
            return;
          }
        }
      }

      this.db.saveNews(id, scope, accountId, title, body, priority, authorId, expiresAt);
      const savedNews = this.db.getNews(id);

      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveNews, {
        s: 'OK:save',
        newsId: id
      }, true));

      // Fan out the change to every connected comm server so it can be pushed
      // to GCS clients in real time (in addition to their periodic resync).
      this.wsServer.broadcast(this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_NewsPush, {
        news: savedNews
      }, true));

      logger.info(`Saved news ${id}`);
    } catch (error) {
      logger.error(`Error saving news: ${error.message}`);

      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveNews, {
        s: 'ERROR:' + error.message
      }, false, error.message));
    }
  }

  /**
   * Handle DeleteNews message (9017) - soft-delete (disable) by default
   */
  async handleDeleteNews(connectionId, message) {
    const payload = this.getPayload(message);
    const { newsId } = payload;

    try {
      logger.info(`DeleteNews request for news ${newsId}`);

      const result = this.db.disableNews(newsId);

      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteNews, {
        s: 'OK:delete',
        newsId: newsId,
        deleted: result.changes
      }, true));

      // Notify connected comm servers so they can remove/hide the item live.
      this.wsServer.broadcast(this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_NewsPush, {
        news: { id: newsId, disabled: 1 }
      }, true));

      logger.info(`Disabled news ${newsId} (changes: ${result.changes})`);
    } catch (error) {
      logger.error(`Error deleting news ${newsId}: ${error.message}`);

      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteNews, {
        s: 'ERROR:' + error.message,
        newsId: newsId
      }, false, error.message));
    }
  }

  /**
   * Handle message by numeric type (mt field)
   */
  async handleMessageByType(connectionId, message) {
    const messageType = message.mt;

    switch (messageType) {
      case CONST_TYPE_AndruavSystem_LoadTasks:
        await this.handleLoadTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_SaveTasks:
        await this.handleSaveTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_DeleteTasks:
        await this.handleDeleteTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_DisableTasks:
        await this.handleDisableTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_UnitOnline:
        await this.handleUnitOnline(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_LoadMission:
        await this.handleLoadMission(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_SaveMission:
        await this.handleSaveMission(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_DeleteMission:
        await this.handleDeleteMission(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_LoadNews:
        await this.handleLoadNews(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_SaveNews:
        await this.handleSaveNews(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_DeleteNews:
        await this.handleDeleteNews(connectionId, message);
        break;
      default:
        logger.warn(`Unknown message type: ${messageType}`);
        this.wsServer.sendError(connectionId, `Unknown message type: ${messageType}`);
    }
  }

  /**
   * Get handlers map - numeric type mapping
   */
  getHandlers() {
    return {
      [CONST_TYPE_AndruavSystem_LoadTasks]: this.handleLoadTasks.bind(this),
      [CONST_TYPE_AndruavSystem_SaveTasks]: this.handleSaveTasks.bind(this),
      [CONST_TYPE_AndruavSystem_DeleteTasks]: this.handleDeleteTasks.bind(this),
      [CONST_TYPE_AndruavSystem_DisableTasks]: this.handleDisableTasks.bind(this),
      [CONST_TYPE_AndruavSystem_UnitOnline]: this.handleUnitOnline.bind(this),
      [CONST_TYPE_AndruavSystem_LoadMission]: this.handleLoadMission.bind(this),
      [CONST_TYPE_AndruavSystem_SaveMission]: this.handleSaveMission.bind(this),
      [CONST_TYPE_AndruavSystem_DeleteMission]: this.handleDeleteMission.bind(this),
      [CONST_TYPE_AndruavSystem_LoadNews]: this.handleLoadNews.bind(this),
      [CONST_TYPE_AndruavSystem_SaveNews]: this.handleSaveNews.bind(this),
      [CONST_TYPE_AndruavSystem_DeleteNews]: this.handleDeleteNews.bind(this)
    };
  }
}

module.exports = MessageHandlers;
