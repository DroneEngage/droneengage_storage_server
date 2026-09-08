const { Colors } = require('droneengage_server_common').helpers.colors;

class Logger {
  constructor() {
    // Use global logger if available (set by server.js)
    this.useGlobal = (global.m_logger !== undefined);
  }

  info(message) {
    if (this.useGlobal && global.m_logger) {
      global.m_logger.Info(message);
    }
    console.log(Colors.BSuccess + '[INFO] ' + Colors.Reset + message);
  }

  error(message) {
    if (this.useGlobal && global.m_logger) {
      global.m_logger.Error(message);
    }
    console.log(Colors.Error + '[ERROR] ' + Colors.Reset + message);
  }

  warn(message) {
    if (this.useGlobal && global.m_logger) {
      global.m_logger.Warn(message);
    }
    console.log(Colors.FgYellow + '[WARN] ' + Colors.Reset + message);
  }

  debug(message) {
    if (this.useGlobal && global.m_logger) {
      global.m_logger.Debug(message);
    }
    // Verbose debug output is gated behind the debug_logging config flag to
    // avoid console log spam (logger.debug fires on every incoming WS message).
    if (global.DEBUG_LOGGING) {
      console.log(Colors.FgCyan + '[DEBUG] ' + Colors.Reset + message);
    }
  }
}

module.exports = new Logger();
