import 'dart:async';

import 'package:flutter/material.dart';

import 'daemon_api.dart';
import 'models.dart';
import 'settings.dart';
import 'solver_case_screen.dart';

enum SolverQueue { pending, taken }

class SolverDesk extends StatefulWidget {
  const SolverDesk({
    super.key,
    required this.settings,
    required this.daemon,
  });

  final SettingsController settings;
  final DaemonApi daemon;

  @override
  State<SolverDesk> createState() => _SolverDeskState();
}

class _SolverDeskState extends State<SolverDesk> {
  final _taken = <String>{};
  final _closed = <String>{};
  List<TradeSnapshot> _trades = const [];
  String? _error;
  var _loading = true;
  var _queue = SolverQueue.pending;
  Timer? _poll;

  UserSettings get _user => widget.settings.settings;

  List<TradeSnapshot> get _pending => [
    for (final trade in _trades)
      if (trade.isDisputed && !_taken.contains(trade.id) && !_closed.contains(trade.id))
        trade,
  ];

  List<TradeSnapshot> get _workspace => [
    for (final trade in _trades)
      if (_taken.contains(trade.id)) trade,
  ];

  @override
  void initState() {
    super.initState();
    _load();
    _poll = Timer.periodic(const Duration(seconds: 5), (_) {
      _fetch(showSpinner: false);
    });
  }

  @override
  void dispose() {
    _poll?.cancel();
    super.dispose();
  }

  Future<void> _load() => _fetch(showSpinner: true);

  Future<void> _fetch({required bool showSpinner}) async {
    if (showSpinner) {
      setState(() {
        _loading = true;
        _error = null;
      });
    }
    try {
      final trades = await widget.daemon.listTrades(_user.daemonUrl);
      if (!mounted) {
        return;
      }
      setState(() {
        _trades = trades;
        _taken.removeWhere((id) => _closed.contains(id));
        _loading = false;
      });
    } catch (err) {
      if (!mounted) {
        return;
      }
      setState(() {
        _error = err.toString();
        _loading = false;
      });
    }
  }

  void _take(TradeSnapshot trade) {
    setState(() {
      _taken.add(trade.id);
      _queue = SolverQueue.taken;
    });
    _open(trade.id);
  }

  Future<void> _open(String id) async {
    final settled = await Navigator.of(context).push<bool>(
      MaterialPageRoute<bool>(
        builder: (_) => SolverCaseScreen(
          settings: widget.settings,
          daemon: widget.daemon,
          tradeId: id,
        ),
      ),
    );
    if (!mounted) {
      return;
    }
    if (settled == true) {
      setState(() {
        _taken.remove(id);
        _closed.add(id);
      });
    }
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    final rows = _queue == SolverQueue.pending ? _pending : _workspace;
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(8, 0, 8, 4),
          child: Row(
            children: [
              _queueTab(SolverQueue.pending, 'Pending (${_pending.length})'),
              _queueTab(SolverQueue.taken, 'In progress (${_workspace.length})'),
            ],
          ),
        ),
        if (_error != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Text(_error!, key: const Key('solver-error')),
          ),
        Expanded(
          child: _loading && _trades.isEmpty
              ? const Center(child: Text('Loading'))
              : rows.isEmpty
              ? Center(
                  child: Text(
                    _queue == SolverQueue.pending
                        ? 'No pending disputes. File one from an order, then pull to refresh.'
                        : 'Take a dispute from Pending.',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.onSurfaceVariant,
                    ),
                  ),
                )
              : ListView.separated(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
                  itemCount: rows.length,
                  separatorBuilder: (_, _) => const SizedBox(height: 12),
                  itemBuilder: (context, index) {
                    final trade = rows[index];
                    return Card(
                      key: Key('solver-trade-${trade.id}'),
                      child: ListTile(
                        title: Text(
                          '${trade.amount} CKB · ${trade.payAmount} ${trade.currency}',
                        ),
                        subtitle: Text(
                          'from ${trade.disputeFrom ?? '?'} · ${trade.disputeReason ?? '-'}',
                        ),
                        trailing: _queue == SolverQueue.pending
                            ? TextButton(
                                key: Key('take-dispute-${trade.id}'),
                                onPressed: () => _take(trade),
                                child: const Text('Take'),
                              )
                            : const Icon(Icons.chevron_right),
                        onTap: _queue == SolverQueue.pending
                            ? () => _take(trade)
                            : () => _open(trade.id),
                      ),
                    );
                  },
                ),
        ),
      ],
    );
  }

  Widget _queueTab(SolverQueue value, String label) {
    final selected = _queue == value;
    final colors = Theme.of(context).colorScheme;
    return Expanded(
      child: InkWell(
        key: Key(value == SolverQueue.pending ? 'solver-pending' : 'solver-taken'),
        onTap: () => setState(() => _queue = value),
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 12),
              child: Text(
                label,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontWeight: FontWeight.w600,
                  color: selected ? colors.onSurface : colors.onSurfaceVariant,
                ),
              ),
            ),
            Container(
              height: 2,
              color: selected ? colors.primary : Colors.transparent,
            ),
          ],
        ),
      ),
    );
  }
}
