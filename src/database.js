const Database = require('better-sqlite3');
const path = require('path');
const fs = require('fs');
const logger = require('./logger');
const serverConfig = require('../js_serverConfig.js');

class DatabaseManager {
  constructor() {
    const config = serverConfig.m_configuration;
    this.config = config;
    this.dbPath = path.resolve(__dirname, '..', config.database.path);
    this.db = null;
  }

  /**
   * Initialize database connection and create tables
   */
  initialize() {
    // Ensure data directory exists
    const dataDir = path.dirname(this.dbPath);
    if (!fs.existsSync(dataDir)) {
      fs.mkdirSync(dataDir, { recursive: true });
    }

    try {
      this.db = new Database(this.dbPath);
      this.db.pragma('journal_mode = WAL');
      this.db.pragma('foreign_keys = ON');
      
      logger.info(`Database initialized at ${this.dbPath}`);
      
      this.createTables();
      this.createIndexes();
      
      return true;
    } catch (error) {
      logger.error(`Database initialization failed: ${error.message}`);
      return false;
    }
  }

  /**
   * Create database tables
   */
  createTables() {
    const tables = [
      // Units table
      `CREATE TABLE IF NOT EXISTS units (
        id TEXT PRIMARY KEY,
        name TEXT,
        comm_server_id TEXT,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        updated_at INTEGER DEFAULT (strftime('%s', 'now'))
      )`,
      
      // Tasks table
      `CREATE TABLE IF NOT EXISTS tasks (
        id TEXT PRIMARY KEY,
        unit_id TEXT NOT NULL,
        name TEXT,
        data TEXT NOT NULL,
        version INTEGER DEFAULT 1,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        updated_at INTEGER DEFAULT (strftime('%s', 'now')),
        disabled INTEGER DEFAULT 0,
        FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
      )`,
      
      // Missions table
      `CREATE TABLE IF NOT EXISTS missions (
        id TEXT PRIMARY KEY,
        unit_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        name TEXT,
        data TEXT NOT NULL,
        version INTEGER DEFAULT 1,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        updated_at INTEGER DEFAULT (strftime('%s', 'now')),
        FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
      )`,
      
      // Offline queue table
      `CREATE TABLE IF NOT EXISTS offline_queue (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        unit_id TEXT NOT NULL,
        task_id TEXT,
        message_type INTEGER NOT NULL,
        message_data TEXT NOT NULL,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        priority INTEGER DEFAULT 0,
        status TEXT DEFAULT 'pending',
        FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
      )`,
      
      // Access log table
      `CREATE TABLE IF NOT EXISTS access_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        unit_id TEXT,
        action TEXT NOT NULL,
        resource_type TEXT,
        resource_id TEXT,
        comm_server_id TEXT,
        created_at INTEGER DEFAULT (strftime('%s', 'now'))
      )`
    ];

    tables.forEach(tableSQL => {
      try {
        this.db.exec(tableSQL);
      } catch (error) {
        logger.error(`Error creating table: ${error.message}`);
      }
    });

    logger.info('Database tables created');

    // Migration: add account_id column to missions table if missing (backwards compatibility)
    try {
      this.db.exec("ALTER TABLE missions ADD COLUMN account_id TEXT NOT NULL DEFAULT '_unknown_'");
      logger.info('Migration: added account_id column to missions table');
    } catch (error) {
      if (error.message.indexOf('duplicate column') === -1) {
        logger.error(`Migration: failed to add account_id to missions: ${error.message}`);
      }
    }
  }

  /**
   * Create database indexes
   */
  createIndexes() {
    const indexes = [
      'CREATE INDEX IF NOT EXISTS idx_tasks_unit_id ON tasks(unit_id)',
      'CREATE INDEX IF NOT EXISTS idx_tasks_disabled ON tasks(disabled)',
      'CREATE INDEX IF NOT EXISTS idx_offline_queue_unit_id ON offline_queue(unit_id)',
      'CREATE INDEX IF NOT EXISTS idx_offline_queue_status ON offline_queue(status)',
      'CREATE INDEX IF NOT EXISTS idx_offline_queue_priority ON offline_queue(priority)',
      'CREATE INDEX IF NOT EXISTS idx_access_log_unit_id ON access_log(unit_id)',
      'CREATE INDEX IF NOT EXISTS idx_access_log_created_at ON access_log(created_at)',
      'CREATE INDEX IF NOT EXISTS idx_missions_unit_id ON missions(unit_id)',
      'CREATE INDEX IF NOT EXISTS idx_missions_account_id ON missions(account_id)'
    ];

    indexes.forEach(indexSQL => {
      try {
        this.db.exec(indexSQL);
      } catch (error) {
        logger.error(`Error creating index: ${error.message}`);
      }
    });

    logger.info('Database indexes created');
  }

  /**
   * Unit operations
   */
  
  // Register or update unit
  upsertUnit(unitId, name, commServerId) {
    const stmt = this.db.prepare(`
      INSERT INTO units (id, name, comm_server_id)
      VALUES (?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        name = excluded.name,
        comm_server_id = excluded.comm_server_id,
        updated_at = strftime('%s', 'now')
    `);
    
    return stmt.run(unitId, name, commServerId);
  }

  // Get unit
  getUnit(unitId) {
    const stmt = this.db.prepare('SELECT * FROM units WHERE id = ?');
    return stmt.get(unitId);
  }

  // Delete unit
  deleteUnit(unitId) {
    const stmt = this.db.prepare('DELETE FROM units WHERE id = ?');
    return stmt.run(unitId);
  }

  /**
   * Task operations
   */

  // Save task
  saveTask(taskId, unitId, name, data) {
    const stmt = this.db.prepare(`
      INSERT INTO tasks (id, unit_id, name, data)
      VALUES (?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        unit_id = excluded.unit_id,
        name = excluded.name,
        data = excluded.data,
        version = version + 1,
        updated_at = strftime('%s', 'now')
    `);
    
    return stmt.run(taskId, unitId, name, JSON.stringify(data));
  }

  // Load tasks for unit
  loadTasks(unitId) {
    const stmt = this.db.prepare(`
      SELECT * FROM tasks 
      WHERE unit_id = ? AND disabled = 0
      ORDER BY created_at DESC
    `);
    
    const tasks = stmt.all(unitId);
    return tasks.map(task => ({
      ...task,
      data: JSON.parse(task.data)
    }));
  }

  // Delete task
  deleteTask(taskId) {
    const stmt = this.db.prepare('DELETE FROM tasks WHERE id = ?');
    return stmt.run(taskId);
  }

  // Disable task
  disableTask(taskId) {
    const stmt = this.db.prepare(`
      UPDATE tasks 
      SET disabled = 1, updated_at = strftime('%s', 'now')
      WHERE id = ?
    `);
    
    return stmt.run(taskId);
  }

  // Get task
  getTask(taskId) {
    const stmt = this.db.prepare('SELECT * FROM tasks WHERE id = ?');
    const task = stmt.get(taskId);
    
    if (task) {
      task.data = JSON.parse(task.data);
    }
    
    return task;
  }

  /**
   * Mission operations
   */

  // Save mission
  saveMission(missionId, unitId, accountId, name, data) {
    const stmt = this.db.prepare(`
      INSERT INTO missions (id, unit_id, account_id, name, data)
      VALUES (?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        unit_id = excluded.unit_id,
        account_id = excluded.account_id,
        name = excluded.name,
        data = excluded.data,
        version = version + 1,
        updated_at = strftime('%s', 'now')
    `);
    
    return stmt.run(missionId, unitId, accountId, name, JSON.stringify(data));
  }

  // Load missions for unit (scoped by account_id)
  loadMissions(unitId, accountId) {
    const stmt = this.db.prepare(`
      SELECT * FROM missions 
      WHERE unit_id = ? AND account_id = ?
      ORDER BY created_at DESC
    `);
    
    const missions = stmt.all(unitId, accountId);
    return missions.map(mission => ({
      ...mission,
      data: JSON.parse(mission.data)
    }));
  }

  // Get mission by ID (scoped by account_id)
  getMission(missionId, accountId) {
    const stmt = this.db.prepare('SELECT * FROM missions WHERE id = ? AND account_id = ?');
    const mission = stmt.get(missionId, accountId);
    
    if (mission) {
      mission.data = JSON.parse(mission.data);
    }
    
    return mission;
  }

  // Delete mission (scoped by account_id)
  deleteMission(missionId, accountId) {
    const stmt = this.db.prepare('DELETE FROM missions WHERE id = ? AND account_id = ?');
    return stmt.run(missionId, accountId);
  }

  /**
   * Offline queue operations
   */

  // Add message to offline queue
  addToQueue(unitId, messageType, messageData, taskId = null, priority = 0) {
    const stmt = this.db.prepare(`
      INSERT INTO offline_queue (unit_id, task_id, message_type, message_data, priority)
      VALUES (?, ?, ?, ?, ?)
    `);
    
    return stmt.run(unitId, taskId, messageType, JSON.stringify(messageData), priority);
  }

  // Get queued messages for unit
  getQueuedMessages(unitId, limit = 100) {
    const stmt = this.db.prepare(`
      SELECT * FROM offline_queue
      WHERE unit_id = ? AND status = 'pending'
      ORDER BY priority DESC, created_at ASC
      LIMIT ?
    `);
    
    const messages = stmt.all(unitId, limit);
    return messages.map(msg => ({
      ...msg,
      message_data: JSON.parse(msg.message_data)
    }));
  }

  // Mark queue message as delivered
  markMessageDelivered(messageId) {
    const stmt = this.db.prepare(`
      UPDATE offline_queue
      SET status = 'delivered'
      WHERE id = ?
    `);
    
    return stmt.run(messageId);
  }

  // Clean up delivered messages (older than 1 day)
  cleanupDeliveredMessages() {
    const stmt = this.db.prepare(`
      DELETE FROM offline_queue
      WHERE status = 'delivered' AND created_at < strftime('%s', 'now') - 86400
    `);
    
    const result = stmt.run();
    logger.info(`Cleaned up ${result.changes} delivered queue messages`);
    return result;
  }

  /**
   * Access logging
   */

  // Get pending queue size for a unit
  getQueueSize(unitId) {
    const stmt = this.db.prepare(`
      SELECT COUNT(*) as count FROM offline_queue 
      WHERE unit_id = ? AND status = 'pending'
    `);
    return stmt.get(unitId).count;
  }

  // Get overall queue statistics
  getQueueStats() {
    const stmt = this.db.prepare(`
      SELECT 
        COUNT(*) as totalMessages,
        COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0) as pendingMessages,
        COALESCE(COUNT(DISTINCT unit_id), 0) as affectedUnits
      FROM offline_queue
      WHERE status = 'pending'
    `);
    return stmt.get();
  }

  // Get queue status for a unit
  getQueueStatus(unitId) {
    const stmt = this.db.prepare(`
      SELECT 
        COUNT(*) as total,
        SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
        SUM(CASE WHEN status = 'delivered' THEN 1 ELSE 0 END) as delivered
      FROM offline_queue
      WHERE unit_id = ?
    `);
    return stmt.get(unitId);
  }

  // Clear queue for a unit
  clearQueue(unitId) {
    const stmt = this.db.prepare(`
      DELETE FROM offline_queue WHERE unit_id = ?
    `);
    return stmt.run(unitId);
  }

  // Log access
  logAccess(unitId, action, resourceType, resourceId, commServerId) {
    const stmt = this.db.prepare(`
      INSERT INTO access_log (unit_id, action, resource_type, resource_id, comm_server_id)
      VALUES (?, ?, ?, ?, ?)
    `);
    
    return stmt.run(unitId, action, resourceType, resourceId, commServerId);
  }

  /**
   * Utility functions
   */

  // Close database connection
  close() {
    if (this.db) {
      this.db.close();
      logger.info('Database connection closed');
    }
  }

  // Get database stats
  getStats() {
    const stats = {
      units: this.db.prepare('SELECT COUNT(*) as count FROM units').get().count,
      tasks: this.db.prepare('SELECT COUNT(*) as count FROM tasks').get().count,
      disabledTasks: this.db.prepare('SELECT COUNT(*) as count FROM tasks WHERE disabled = 1').get().count,
      missions: this.db.prepare('SELECT COUNT(*) as count FROM missions').get().count,
      queuedMessages: this.db.prepare("SELECT COUNT(*) as count FROM offline_queue WHERE status = 'pending'").get().count,
      dbSize: fs.statSync(this.dbPath).size
    };
    
    return stats;
  }
}

module.exports = DatabaseManager;
