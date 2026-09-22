import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:image_picker/image_picker.dart';

import 'amounts.dart';
import 'daemon_api.dart';
import 'fiber_api.dart';
import 'models.dart';
import 'settings.dart';

const Duration _orderPollInterval = Duration(seconds: 5);
const Duration _clockTick = Duration(seconds: 1);

class TradeScreen extends StatefulWidget {
  const TradeScreen({
    super.key,
    required this.settings,
    required this.daemon,
    required this.fiber,
    required this.tradeId,
    this.pickProof,
  });

  final SettingsController settings;
  final DaemonApi daemon;
  final FiberApi fiber;
  final String tradeId;
  final Future<PickedProof?> Function()? pickProof;

  @override
  State<TradeScreen> createState() => _TradeScreenState();
}

class _TradeScreenState extends State<TradeScreen> {
  final TextEditingController _chat = TextEditingController();
  final TextEditingController _reason = TextEditingController();
  TradeSnapshot? _trade;
  PickedProof? _picked;
  Uint8List? _proofBytes;
  String? _error;
  var _loading = true;
  var _busy = false;
  Timer? _orderPoll;
  Timer? _clock;

  UserSettings get _user => widget.settings.settings;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _orderPoll?.cancel();
    _clock?.cancel();
    _chat.dispose();
    _reason.dispose();
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
      _syncPolls(trade);
      await _maybeLoadProof(trade);
    } catch (err) {
      if (!mounted) {
        return;
      }
      setState(() {
        _error = err.toString();
        _loading = false;
      });
      _syncPolls(null);
    }
  }

  Future<void> _pollOrder() async {
    if (_busy || _loading) {
      return;
    }
    try {
      final trade = await widget.daemon.fetchTrade(
        _user.daemonUrl,
        widget.tradeId,
      );
      if (!mounted) {
        return;
      }
      setState(() => _trade = trade);
      _syncPolls(trade);
      await _maybeLoadProof(trade);
    } catch (_) {}
  }

  void _syncPolls(TradeSnapshot? trade) {
    final shouldPoll = trade?.watchesHoldExpiry ?? false;
    if (shouldPoll) {
      _orderPoll ??= Timer.periodic(_orderPollInterval, (_) {
        _pollOrder();
      });
    } else {
      _orderPoll?.cancel();
      _orderPoll = null;
    }
    final hasDeadline = _deadlineOf(trade) != null && shouldPoll;
    if (hasDeadline) {
      _clock ??= Timer.periodic(_clockTick, (_) {
        if (mounted) {
          setState(() {});
        }
      });
    } else {
      _clock?.cancel();
      _clock = null;
    }
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
      _syncPolls(trade);
      await _maybeLoadProof(trade);
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

  Future<void> _maybeLoadProof(TradeSnapshot trade) async {
    if (!trade.hasProof) {
      if (_proofBytes != null) {
        setState(() => _proofBytes = null);
      }
      return;
    }
    if (_proofBytes != null) {
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

  Future<String> _buyerInvoice() async {
    final trade = _trade;
    if (trade == null) {
      throw const DaemonException('trade is not loaded');
    }
    return widget.fiber.newInvoice(
      _user.fiberRpc,
      amountHex: shannonHex(trade.amount),
      description: 'twine payout ${trade.id}',
    );
  }

  Future<void> _chooseProof() async {
    final picked = widget.pickProof != null
        ? await widget.pickProof!()
        : await _pickFromGallery();
    if (!mounted || picked == null) {
      return;
    }
    setState(() => _picked = picked);
  }

  Future<PickedProof?> _pickFromGallery() async {
    final file = await ImagePicker().pickImage(source: ImageSource.gallery);
    if (file == null) {
      return null;
    }
    final bytes = await file.readAsBytes();
    final name = file.name.toLowerCase();
    final contentType = name.endsWith('.png')
        ? 'image/png'
        : 'image/jpeg';
    return PickedProof(bytes: bytes, contentType: contentType);
  }

  Future<void> _copy(String value) async {
    await Clipboard.setData(ClipboardData(text: value));
    if (!mounted) {
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Copied')),
    );
  }

  DateTime? _deadlineOf(TradeSnapshot? trade) {
    if (trade == null) {
      return null;
    }
    final raw = trade.isWaitingHold ? trade.acceptBy : trade.payBy;
    if (raw == null || raw.isEmpty) {
      return null;
    }
    return DateTime.tryParse(raw);
  }

  String? get _deadlineRemaining {
    final trade = _trade;
    final deadline = _deadlineOf(trade);
    if (deadline == null) {
      return null;
    }
    if (!(trade?.isWaitingHold ?? false) && !(trade?.isWaitingFiat ?? false)) {
      return null;
    }
    final left = deadline.difference(DateTime.now());
    if (left.isNegative) {
      return '0:00';
    }
    return '${left.inMinutes}:${(left.inSeconds % 60).toString().padLeft(2, '0')}';
  }

  String _sideLabel(TradeSnapshot trade) {
    if (trade.isLister(_user.pubkey)) {
      return 'You listed this';
    }
    if (trade.isTaker(_user.pubkey)) {
      return 'You took this';
    }
    return 'Watching this trade';
  }

  String _nextStep(TradeSnapshot trade, bool listed, bool took) {
    if (trade.isWaitingHold && listed) {
      return 'Accept the order to lock CKB on Fiber. Until then the hold is Open and either of you can cancel.';
    }
    if (trade.isWaitingHold && took) {
      return 'Waiting for the seller to accept. The hold invoice is still Open.';
    }
    if (trade.isWaitingFiat && took) {
      return 'Pay the seller with the handle below, then upload a receipt.';
    }
    if (trade.isWaitingFiat && listed) {
      return 'Buyer is paying you. CKB stays locked on Fiber until you release.';
    }
    if (trade.isPayWindowClosed && listed) {
      return 'Buyer did not pay. Your CKB returns when the hold expires.';
    }
    if (trade.isPayWindowClosed && took) {
      return 'Payment window closed. The hold cannot be cancelled while Received.';
    }
    if ((trade.isFiatSent || trade.isReleasing) && listed) {
      return 'Check the receipt, then release. Twine pays the buyer invoice and settles the hold.';
    }
    if ((trade.isFiatSent || trade.isReleasing) && took) {
      return 'Waiting for the seller to release. Twine will pay your invoice, then settle.';
    }
    if (trade.isLeg2Failed && took) {
      return 'Twine could not pay your invoice. Submit a new one to retry. The hold stays Received.';
    }
    if (trade.isLeg2Failed && listed) {
      return 'Payment to the buyer failed. The hold stays Received until they retry or the TLC expires.';
    }
    if (trade.isDisputed) {
      return 'Appeal is open. Twine awards only after this filing.';
    }
    if (trade.isSettled) {
      return 'Completed. Twine paid the buyer and settled the hold.';
    }
    if (trade.isCancelled) {
      return 'Cancelled while the hold was Open. Reserved CKB is back on the ad.';
    }
    if (trade.isExpired) {
      return 'Hold expired. Seller refunded because the TLC expired.';
    }
    return trade.statusFor(_user.pubkey);
  }

  int _stepIndex(TradeSnapshot? trade) {
    if (trade == null || trade.isWaitingHold || trade.isCancelled) {
      return 0;
    }
    if (trade.isWaitingFiat || trade.isPayWindowClosed) {
      return 1;
    }
    return 2;
  }

  @override
  Widget build(BuildContext context) {
    final trade = _trade;
    final listed = trade?.isLister(_user.pubkey) ?? false;
    final took = trade?.isTaker(_user.pubkey) ?? false;
    final operator = _user.operatorTools;
    final canChat = (took || listed) && (trade?.canChat ?? false) && !_busy;
    final chatReady = canChat && _chat.text.trim().isNotEmpty;
    final canAppeal =
        (took || listed) && (trade?.canOpenDispute ?? false) && !_busy;
    final appealReady = canAppeal && _reason.text.trim().isNotEmpty;
    final stateLabel = _loading ? 'Loading' : (trade?.state ?? 'Idle');
    final colors = Theme.of(context).colorScheme;

    return Scaffold(
      appBar: AppBar(title: const Text('Order')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          _StepHeader(current: _stepIndex(trade)),
          const SizedBox(height: 16),
          if (trade != null) Text(_sideLabel(trade)),
          if (trade != null) ...[
            const SizedBox(height: 8),
            Text(
              _nextStep(trade, listed, took),
              style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                color: colors.onSurfaceVariant,
              ),
            ),
          ],
          if (_deadlineRemaining != null)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text(
                trade?.isWaitingHold ?? false
                    ? 'Accept window: $_deadlineRemaining'
                    : 'Fiat window: $_deadlineRemaining',
                key: const Key('fiat-timer'),
              ),
            ),
          const SizedBox(height: 16),
          if (listed && (trade?.isWaitingHold ?? false)) ...[
            FilledButton(
              key: const Key('lock'),
              onPressed: _busy
                  ? null
                  : () => _run(() async {
                      final invoice = trade?.invoiceAddress;
                      if (invoice == null || invoice.isEmpty) {
                        throw const DaemonException('missing hold invoice');
                      }
                      await widget.fiber.sendPayment(_user.fiberRpc, invoice);
                      return widget.daemon.markLocked(
                        _user.daemonUrl,
                        widget.tradeId,
                      );
                    }),
              child: const Text('Accept order'),
            ),
            const SizedBox(height: 8),
            OutlinedButton(
              key: const Key('reject-order'),
              onPressed: _busy
                  ? null
                  : () => _run(
                      () => widget.daemon.cancelTrade(
                        _user.daemonUrl,
                        widget.tradeId,
                        from: 'lister',
                      ),
                    ),
              child: const Text('Reject'),
            ),
          ],
          if (took && (trade?.isWaitingHold ?? false))
            OutlinedButton(
              key: const Key('cancel-order'),
              onPressed: _busy
                  ? null
                  : () => _run(
                      () => widget.daemon.cancelTrade(
                        _user.daemonUrl,
                        widget.tradeId,
                        from: 'taker',
                      ),
                    ),
              child: const Text('Cancel order'),
            ),
          if (trade != null) ...[
            const SizedBox(height: 16),
            _PayCard(
              trade: trade,
              onCopy: () => _copy(trade.paymentMethod),
            ),
          ],
          if (took && (trade?.isWaitingFiat ?? false)) ...[
            const SizedBox(height: 8),
            OutlinedButton(
              key: const Key('pick-proof'),
              onPressed: _busy ? null : _chooseProof,
              child: Text(
                _picked == null ? 'Upload receipt' : 'Receipt selected',
              ),
            ),
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('fiat-sent'),
              onPressed: _busy || _picked == null
                  ? null
                  : () => _run(() async {
                      final invoice = await _buyerInvoice();
                      final picked = _picked!;
                      return widget.daemon.fiatSent(
                        _user.daemonUrl,
                        widget.tradeId,
                        invoice: invoice,
                        proofB64: base64Encode(picked.bytes),
                        contentType: picked.contentType,
                      );
                    }),
              child: const Text('I have paid'),
            ),
          ],
          if (_proofBytes != null) ...[
            const SizedBox(height: 12),
            const Text('Payment proof', key: Key('proof-heading')),
            const SizedBox(height: 8),
            Image.memory(
              _proofBytes!,
              key: const Key('proof-image'),
              height: 180,
              fit: BoxFit.contain,
            ),
          ],
          if (listed &&
              ((trade?.isFiatSent ?? false) || (trade?.isReleasing ?? false))) ...[
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('release'),
              onPressed: _busy
                  ? null
                  : () => _run(
                      () => widget.daemon.release(
                        _user.daemonUrl,
                        widget.tradeId,
                      ),
                    ),
              child: Text(
                (trade?.isReleasing ?? false)
                    ? 'Payment received (continue path A)'
                    : 'Release CKB',
              ),
            ),
          ],
          if (trade?.isLeg2Failed ?? false) ...[
            const SizedBox(height: 12),
            const Text(
              'Path B: payment to the buyer failed. Submit a new invoice to retry. '
              'If the buyer never returns, the seller is refunded when the TLC expires. '
              'Do not cancel the held invoice.',
              key: Key('leg2-failed-message'),
            ),
            if (took) ...[
              const SizedBox(height: 8),
              FilledButton(
                key: const Key('retry'),
                onPressed: _busy
                    ? null
                    : () => _run(() async {
                        final invoice = await _buyerInvoice();
                        return widget.daemon.retry(
                          _user.daemonUrl,
                          widget.tradeId,
                          invoice: invoice,
                        );
                      }),
                child: const Text('Retry with new invoice'),
              ),
            ],
          ],
          if ((took || listed) && (trade?.canChat ?? false)) ...[
            const SizedBox(height: 12),
            const Text('Chat', key: Key('dispute-chat-heading')),
            if (trade!.chat.isEmpty)
              const Text('No chat lines yet.', key: Key('chat-empty'))
            else
              for (final line in trade.chat)
                Padding(
                  padding: const EdgeInsets.only(bottom: 4),
                  child: Text(
                    '${line.at}  ${line.from}: ${line.text}',
                    key: const Key('chat-line'),
                  ),
                ),
            TextField(
              key: const Key('chat-input'),
              controller: _chat,
              decoration: const InputDecoration(labelText: 'Chat line'),
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('post-chat'),
              onPressed: chatReady
                  ? () {
                      final text = _chat.text;
                      _chat.clear();
                      _run(
                        () => widget.daemon.postChat(
                          _user.daemonUrl,
                          widget.tradeId,
                          from: took ? 'taker' : 'lister',
                          text: text,
                        ),
                      );
                    }
                  : null,
              child: const Text('Post chat line'),
            ),
          ],
          if (canAppeal) ...[
            const SizedBox(height: 8),
            TextField(
              key: const Key('dispute-reason'),
              controller: _reason,
              decoration: const InputDecoration(labelText: 'Appeal reason'),
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: 8),
            OutlinedButton(
              key: const Key('open-dispute'),
              onPressed: appealReady
                  ? () => _run(
                      () => widget.daemon.openDispute(
                        _user.daemonUrl,
                        widget.tradeId,
                        from: took ? 'taker' : 'lister',
                        reason: _reason.text,
                      ),
                    )
                  : null,
              child: const Text('File dispute'),
            ),
          ],
          if (trade?.isDisputed ?? false) ...[
            const SizedBox(height: 12),
            if (trade!.disputeReason != null)
              Text(
                'Appeal (${trade.disputeFrom ?? 'party'}): ${trade.disputeReason}',
                key: const Key('dispute-reason-line'),
              ),
            if (operator) ...[
              const SizedBox(height: 8),
              FilledButton(
                key: const Key('award-buyer'),
                onPressed: _busy
                    ? null
                    : () => _run(() async {
                        final invoice = took
                            ? await _buyerInvoice()
                            : (trade.buyerInvoice ??
                                (throw const DaemonException(
                                  'buyer must submit a payout invoice first',
                                )));
                        return widget.daemon.awardBuyer(
                          _user.daemonUrl,
                          widget.tradeId,
                          invoice: invoice,
                        );
                      }),
                child: const Text('Award buyer'),
              ),
              const SizedBox(height: 8),
              FilledButton(
                key: const Key('award-seller'),
                onPressed: _busy
                    ? null
                    : () => _run(
                        () => widget.daemon.awardSeller(
                          _user.daemonUrl,
                          widget.tradeId,
                        ),
                      ),
                child: const Text('Award seller'),
              ),
            ],
            if (trade.sellerWinsLogged)
              const Padding(
                padding: EdgeInsets.only(top: 8),
                child: Text(
                  'Seller wins: hold stays Received. Seller is refunded when the TLC expires. '
                  'No settle_invoice and no cancel_invoice.',
                  key: Key('seller-wins-message'),
                ),
              ),
          ],
          if (trade?.isExpired ?? false) ...[
            const SizedBox(height: 12),
            Text(
              trade!.pathDExpiredLogged
                  ? 'Path D: hold Expired. Seller payment failed back; seller refunded because the TLC expired. '
                      'settle_invoice fails; cancel_invoice was not called.'
                  : 'Path D: hold Expired. Seller is refunded because the TLC expired.',
              key: const Key('path-d-expired-message'),
            ),
          ],
          const SizedBox(height: 16),
          Text(stateLabel, key: const Key('order-state')),
          if (trade != null) ...[
            Text('Amount: ${trade.amount} CKB'),
            Text('Currency: ${trade.currency}'),
            Text('Price: ${trade.price} ${trade.currency}/CKB'),
            Text('You pay: ${trade.payAmount}'),
            Text('Payment: ${trade.paymentMethod}'),
            if (trade.invoiceStatus != null) Text('Invoice: ${trade.invoiceStatus}'),
            if (trade.paymentHash != null) Text('H: ${trade.paymentHash}'),
            if (trade.invoiceAddress != null)
              SelectableText(
                'Invoice address:\n${trade.invoiceAddress}',
                key: const Key('invoice-address'),
              ),
          ],
          if (_error != null) ...[
            const SizedBox(height: 8),
            Text(_error!, key: const Key('error-message')),
          ],
          const SizedBox(height: 16),
          const Text('Log'),
          const SizedBox(height: 8),
          if (trade == null || trade.log.isEmpty)
            const Text('No log lines yet.')
          else
            for (final line in trade.log)
              Padding(
                padding: const EdgeInsets.only(bottom: 4),
                child: Text(
                  '${line.at}  ${line.text}',
                  key: const Key('order-log'),
                ),
              ),
        ],
      ),
    );
  }
}

class _StepHeader extends StatelessWidget {
  const _StepHeader({required this.current});

  final int current;

  static const labels = ['Accept', 'Pay', 'Release'];

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return Row(
      children: [
        for (var index = 0; index < labels.length; index++) ...[
          if (index > 0)
            Expanded(
              child: Container(
                height: 1,
                color: index <= current ? colors.primary : colors.outlineVariant,
              ),
            ),
          Column(
            children: [
              CircleAvatar(
                radius: 12,
                backgroundColor: index <= current
                    ? colors.primary
                    : colors.surfaceContainerHighest,
                foregroundColor: index <= current
                    ? colors.onPrimary
                    : colors.onSurfaceVariant,
                child: Text('${index + 1}', style: const TextStyle(fontSize: 12)),
              ),
              const SizedBox(height: 4),
              Text(
                labels[index],
                style: Theme.of(context).textTheme.labelMedium?.copyWith(
                  color: index <= current ? colors.onSurface : colors.onSurfaceVariant,
                ),
              ),
            ],
          ),
        ],
      ],
    );
  }
}

class _PayCard extends StatelessWidget {
  const _PayCard({required this.trade, required this.onCopy});

  final TradeSnapshot trade;
  final VoidCallback onCopy;

  @override
  Widget build(BuildContext context) {
    return Card(
      key: const Key('pay-card'),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '${trade.payAmount} ${trade.currency}',
              style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(height: 4),
            Text('${trade.amount} CKB · ${trade.price} ${trade.currency}/CKB'),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: Text(
                    trade.paymentMethod,
                    key: const Key('payment-handle'),
                  ),
                ),
                IconButton(
                  key: const Key('copy-payment'),
                  onPressed: onCopy,
                  icon: const Icon(Icons.copy),
                  tooltip: 'Copy payment handle',
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
