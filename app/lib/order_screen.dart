import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;

import 'daemon_api.dart';

enum Role { seller, buyer, solver }

String roleLabel(Role role) {
  switch (role) {
    case Role.seller:
      return 'Seller';
    case Role.buyer:
      return 'Buyer';
    case Role.solver:
      return 'Solver';
  }
}

String defaultDaemonUrl() {
  if (!kIsWeb && defaultTargetPlatform == TargetPlatform.android) {
    return 'http://10.0.2.2:8080';
  }
  return 'http://127.0.0.1:8080';
}

const Duration _fiatWindow = Duration(minutes: 3);

class OrderScreen extends StatefulWidget {
  const OrderScreen({super.key, this.client, this.initialUrl});

  final http.Client? client;
  final String? initialUrl;

  @override
  State<OrderScreen> createState() => _OrderScreenState();
}

class _OrderScreenState extends State<OrderScreen> {
  late final DaemonApi _api = DaemonApi(client: widget.client);
  late final TextEditingController _url = TextEditingController(
    text: widget.initialUrl ?? defaultDaemonUrl(),
  );
  final TextEditingController _amount = TextEditingController();

  Role _role = Role.seller;
  OrderSnapshot? _order;
  String? _error;
  bool _loading = true;
  bool _busy = false;
  DateTime? _fiatDeadline;
  Timer? _fiatTicker;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _fiatTicker?.cancel();
    _url.dispose();
    _amount.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final order = await _api.fetchOrder(_url.text);
      if (!mounted) {
        return;
      }
      setState(() {
        _order = order;
        _loading = false;
        if (order.isWaitingFiat && _fiatDeadline == null) {
          _startFiatTimer();
        }
        if (!order.isWaitingFiat) {
          _clearFiatTimer();
        }
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

  Future<void> _run(Future<OrderSnapshot> Function() action) async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final order = await action();
      if (!mounted) {
        return;
      }
      setState(() {
        _order = order;
        _busy = false;
        if (order.isWaitingFiat) {
          _startFiatTimer();
        } else {
          _clearFiatTimer();
        }
      });
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

  void _startFiatTimer() {
    _fiatDeadline ??= DateTime.now().add(_fiatWindow);
    _fiatTicker?.cancel();
    _fiatTicker = Timer.periodic(const Duration(seconds: 1), (_) {
      if (!mounted) {
        return;
      }
      setState(() {});
    });
  }

  void _clearFiatTimer() {
    _fiatTicker?.cancel();
    _fiatTicker = null;
    _fiatDeadline = null;
  }

  String? get _fiatRemaining {
    final deadline = _fiatDeadline;
    if (deadline == null || !(_order?.isWaitingFiat ?? false)) {
      return null;
    }
    final left = deadline.difference(DateTime.now());
    if (left.isNegative) {
      return '0:00';
    }
    final minutes = left.inMinutes;
    final seconds = left.inSeconds % 60;
    return '$minutes:${seconds.toString().padLeft(2, '0')}';
  }

  @override
  Widget build(BuildContext context) {
    final order = _order;
    final open = order?.isOpen ?? false;
    final canCreate =
        !_loading && !_busy && !open && _amount.text.trim().isNotEmpty;
    final stateLabel = _loading ? 'Loading' : (order?.state ?? 'Idle');
    final seller = _role == Role.seller;
    final buyer = _role == Role.buyer;

    return Scaffold(
      appBar: AppBar(title: const Text('Twine')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          Text('Playing as ${roleLabel(_role)}'),
          const SizedBox(height: 12),
          SegmentedButton<Role>(
            segments: const [
              ButtonSegment(value: Role.seller, label: Text('Seller')),
              ButtonSegment(value: Role.buyer, label: Text('Buyer')),
              ButtonSegment(value: Role.solver, label: Text('Solver')),
            ],
            selected: {_role},
            onSelectionChanged: (selected) {
              setState(() => _role = selected.first);
            },
          ),
          const SizedBox(height: 16),
          TextField(
            key: const Key('daemon-url'),
            controller: _url,
            decoration: const InputDecoration(
              labelText: 'Daemon',
              helperText:
                  'iOS simulator: 127.0.0.1. Android emulator: 10.0.2.2',
            ),
            keyboardType: TextInputType.url,
          ),
          const SizedBox(height: 12),
          TextField(
            key: const Key('amount'),
            controller: _amount,
            decoration: const InputDecoration(labelText: 'Amount (CKB)'),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
            onChanged: (_) => setState(() {}),
          ),
          const SizedBox(height: 12),
          FilledButton(
            key: const Key('create-order'),
            onPressed: canCreate
                ? () => _run(() => _api.createOrder(_url.text, _amount.text))
                : null,
            child: Text(_busy ? 'Working...' : 'Create order'),
          ),
          if (order?.isPending ?? false) ...[
            const SizedBox(height: 8),
            OutlinedButton(
              key: const Key('demo-cancel'),
              onPressed: _busy
                  ? null
                  : () => _run(() => _api.demoCancel(_url.text)),
              child: const Text('Demo cancel unpaid invoice'),
            ),
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('create-hold'),
              onPressed: _busy
                  ? null
                  : () => _run(() => _api.createHold(_url.text)),
              child: const Text('Create hold invoice'),
            ),
          ],
          if (seller && (order?.isWaitingHold ?? false)) ...[
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('lock'),
              onPressed: _busy
                  ? null
                  : () => _run(() => _api.lock(_url.text)),
              child: const Text('Lock'),
            ),
          ],
          if (seller &&
              (order?.isHeld == true ||
                  order?.isWaitingFiat == true ||
                  order?.isFiatSent == true ||
                  order?.isReleasing == true)) ...[
            const SizedBox(height: 8),
            OutlinedButton(
              key: const Key('try-cancel'),
              onPressed: _busy
                  ? null
                  : () => _run(() => _api.tryCancel(_url.text)),
              child: const Text('Try cancel (skipped after Received)'),
            ),
          ],
          if (buyer && (order?.isHeld ?? false)) ...[
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('accept'),
              onPressed: _busy
                  ? null
                  : () => _run(() => _api.accept(_url.text)),
              child: const Text('Accept'),
            ),
          ],
          if (buyer && (order?.isWaitingFiat ?? false)) ...[
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('fiat-sent'),
              onPressed: _busy
                  ? null
                  : () => _run(() => _api.fiatSent(_url.text)),
              child: const Text('Fiat sent'),
            ),
          ],
          if (seller &&
              ((order?.isFiatSent ?? false) ||
                  (order?.isReleasing ?? false))) ...[
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('release'),
              onPressed: _busy
                  ? null
                  : () => _run(() => _api.release(_url.text)),
              child: Text(
                (order?.isReleasing ?? false)
                    ? 'Release (continue path A)'
                    : 'Release',
              ),
            ),
          ],
          const SizedBox(height: 16),
          Text(stateLabel, key: const Key('order-state')),
          if (order?.amount != null) Text('Amount: ${order!.amount} CKB'),
          if (order?.invoiceStatus != null)
            Text('Invoice: ${order!.invoiceStatus}'),
          if (order?.paymentHash != null) Text('H: ${order!.paymentHash}'),
          if (order?.invoiceAddress != null) ...[
            const SizedBox(height: 4),
            SelectableText(
              'Invoice address:\n${order!.invoiceAddress}',
              key: const Key('invoice-address'),
            ),
          ],
          if (_fiatRemaining != null)
            Text(
              'Fiat window: $_fiatRemaining',
              key: const Key('fiat-timer'),
            ),
          if (_error != null) ...[
            const SizedBox(height: 8),
            Text(_error!),
          ],
          const SizedBox(height: 16),
          const Text('Log'),
          const SizedBox(height: 8),
          if (order == null || order.log.isEmpty)
            const Text('No log lines yet.')
          else
            for (final line in order.log)
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
