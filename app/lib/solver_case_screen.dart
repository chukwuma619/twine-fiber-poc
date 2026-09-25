import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';

import 'daemon_api.dart';
import 'models.dart';
import 'settings.dart';

class SolverCaseScreen extends StatefulWidget {
  const SolverCaseScreen({
    super.key,
    required this.settings,
    required this.daemon,
    required this.tradeId,
  });

  final SettingsController settings;
  final DaemonApi daemon;
  final String tradeId;

  @override
  State<SolverCaseScreen> createState() => _SolverCaseScreenState();
}

class _SolverCaseScreenState extends State<SolverCaseScreen> {
  final TextEditingController _chat = TextEditingController();
  TradeSnapshot? _trade;
  Uint8List? _proofBytes;
  String? _error;
  var _loading = true;
  var _busy = false;
  var _party = 'taker';
  Timer? _poll;

  UserSettings get _user => widget.settings.settings;

  List<ChatLine> get _visibleChat {
    final trade = _trade;
    if (trade == null) {
      return const [];
    }
    return [
      for (final line in trade.chat)
        if (line.from == 'admin' ||
            line.from == _party ||
            (_party == 'taker' && line.from == 'buyer') ||
            (_party == 'lister' && line.from == 'seller'))
          line,
    ];
  }

  @override
  void initState() {
    super.initState();
    _load();
    _poll = Timer.periodic(const Duration(seconds: 5), (_) {
      if (!_busy) {
        _refresh();
      }
    });
  }

  @override
  void dispose() {
    _poll?.cancel();
    _chat.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final trade = await widget.daemon.fetchTrade(
        _user.daemonUrl,
        widget.tradeId,
      );
      if (!mounted) {
        return;
      }
      setState(() {
        _trade = trade;
        _loading = false;
      });
      await _maybeLoadProof(trade);
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

  Future<void> _refresh() async {
    try {
      final trade = await widget.daemon.fetchTrade(
        _user.daemonUrl,
        widget.tradeId,
      );
      if (!mounted) {
        return;
      }
      setState(() => _trade = trade);
      await _maybeLoadProof(trade);
    } catch (_) {}
  }

  Future<void> _maybeLoadProof(TradeSnapshot trade) async {
    if (!trade.hasProof || _proofBytes != null) {
      return;
    }
    try {
      final proof = await widget.daemon.fetchProof(
        _user.daemonUrl,
        widget.tradeId,
      );
      if (!mounted) {
        return;
      }
      setState(() {
        _proofBytes = Uint8List.fromList(proof.bytes);
      });
    } catch (_) {}
  }

  Future<void> _run(Future<TradeSnapshot> Function() action) async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final trade = await action();
      if (!mounted) {
        return;
      }
      setState(() {
        _trade = trade;
        _busy = false;
      });
      await _maybeLoadProof(trade);
      if (trade.isSettled || trade.sellerWinsLogged) {
        if (mounted) {
          Navigator.of(context).pop(true);
        }
      }
    } catch (err) {
      if (!mounted) {
        return;
      }
      setState(() {
        _error = err.toString();
        _busy = false;
      });
    }
  }

  Future<void> _confirmAward({required bool payTaker}) async {
    final trade = _trade;
    if (trade == null) {
      return;
    }
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(payTaker ? 'Pay taker?' : 'Refund lister?'),
        content: Text(
          payTaker
              ? 'Pays the stored taker invoice, then settle_invoice (path A). If that pay fails the trade stays Disputed.'
              : 'Does not settle. The hold stays Received until TLC expiry (path D).',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('No'),
          ),
          FilledButton(
            key: const Key('confirm-award'),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Yes'),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) {
      return;
    }
    if (payTaker) {
      final invoice = trade.buyerInvoice?.trim();
      if (invoice == null || invoice.isEmpty) {
        setState(() {
          _error = 'no taker invoice on this trade';
        });
        return;
      }
      await _run(
        () => widget.daemon.awardBuyer(
          _user.daemonUrl,
          widget.tradeId,
          invoice: invoice,
        ),
      );
    } else {
      await _run(
        () => widget.daemon.awardSeller(_user.daemonUrl, widget.tradeId),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final trade = _trade;
    return Scaffold(
      appBar: AppBar(
        title: Text(trade == null ? 'Dispute' : 'Dispute · ${trade.id}'),
      ),
      body: _loading && trade == null
          ? const Center(child: Text('Loading'))
          : ListView(
              padding: const EdgeInsets.fromLTRB(16, 16, 16, 32),
              children: [
                if (trade != null) ...[
                  Text(
                    '${trade.amount} CKB · ${trade.payAmount} ${trade.currency} @ ${trade.price} ${trade.currency}/CKB',
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                  const SizedBox(height: 8),
                  Text('State: ${trade.state}  invoice: ${trade.invoiceStatus ?? '-'}'),
                  Text('Payment: ${trade.paymentMethod}'),
                  Text(
                    'Opened by ${trade.disputeFrom ?? '?'}: ${trade.disputeReason ?? '-'}',
                    key: const Key('solver-reason'),
                  ),
                  const SizedBox(height: 8),
                  Text('Taker  ${_short(trade.taker)}'),
                  Text('Lister  ${_short(trade.pubkey)}'),
                  const SizedBox(height: 16),
                  const Text('Receipt', style: TextStyle(fontWeight: FontWeight.w600)),
                  const SizedBox(height: 8),
                  if (_proofBytes != null)
                    Image.memory(
                      _proofBytes!,
                      key: const Key('solver-proof'),
                      height: 220,
                      fit: BoxFit.contain,
                    )
                  else
                    Text(
                      trade.hasProof ? 'Loading receipt…' : 'No receipt uploaded.',
                      style: TextStyle(
                        color: Theme.of(context).colorScheme.onSurfaceVariant,
                      ),
                    ),
                  const SizedBox(height: 16),
                  SegmentedButton<String>(
                    segments: const [
                      ButtonSegment(value: 'taker', label: Text('Taker')),
                      ButtonSegment(value: 'lister', label: Text('Lister')),
                    ],
                    selected: {_party},
                    onSelectionChanged: (next) {
                      setState(() => _party = next.first);
                    },
                  ),
                  const SizedBox(height: 12),
                  const Text('Chat', style: TextStyle(fontWeight: FontWeight.w600)),
                  const SizedBox(height: 8),
                  if (_visibleChat.isEmpty)
                    const Text('No messages yet.', key: Key('solver-chat-empty'))
                  else
                    for (final line in _visibleChat)
                      Padding(
                        padding: const EdgeInsets.only(bottom: 4),
                        child: Text(
                          '${line.at}  ${line.from}: ${line.text}',
                          key: const Key('solver-chat-line'),
                        ),
                      ),
                  TextField(
                    key: const Key('solver-chat-input'),
                    controller: _chat,
                    decoration: const InputDecoration(
                      labelText: 'Message as admin',
                    ),
                    onChanged: (_) => setState(() {}),
                  ),
                  const SizedBox(height: 8),
                  FilledButton.tonal(
                    key: const Key('solver-post-chat'),
                    onPressed: _busy || _chat.text.trim().isEmpty
                        ? null
                        : () {
                            final text = _chat.text;
                            _chat.clear();
                            _run(
                              () => widget.daemon.postChat(
                                _user.daemonUrl,
                                widget.tradeId,
                                from: 'admin',
                                text: text,
                              ),
                            );
                          },
                    child: const Text('Send'),
                  ),
                  const SizedBox(height: 24),
                  FilledButton(
                    key: const Key('award-buyer'),
                    onPressed: _busy || !(trade.isDisputed)
                        ? null
                        : () => _confirmAward(payTaker: true),
                    child: const Text('Pay taker'),
                  ),
                  const SizedBox(height: 8),
                  OutlinedButton(
                    key: const Key('award-seller'),
                    onPressed: _busy || !(trade.isDisputed)
                        ? null
                        : () => _confirmAward(payTaker: false),
                    child: const Text('Refund lister'),
                  ),
                ],
                if (_error != null) ...[
                  const SizedBox(height: 12),
                  Text(_error!, key: const Key('solver-case-error')),
                ],
              ],
            ),
    );
  }

  String _short(String key) {
    final text = key.trim();
    if (text.length <= 18) {
      return text;
    }
    return '${text.substring(0, 10)}…${text.substring(text.length - 6)}';
  }
}
