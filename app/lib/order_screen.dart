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
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
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

  Future<void> _create() async {
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      final order = await _api.createOrder(_url.text, _amount.text);
      if (!mounted) {
        return;
      }
      setState(() {
        _order = order;
        _saving = false;
      });
    } catch (err) {
      if (!mounted) {
        return;
      }
      setState(() {
        _error = err.toString();
        _saving = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final pending = _order?.isPending ?? false;
    final canCreate =
        !_loading && !_saving && !pending && _amount.text.trim().isNotEmpty;
    final stateLabel = _loading ? 'Loading' : (_order?.state ?? 'Idle');

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
            onPressed: canCreate ? _create : null,
            child: Text(_saving ? 'Creating...' : 'Create order'),
          ),
          const SizedBox(height: 16),
          Text(stateLabel, key: const Key('order-state')),
          if (_order?.amount != null) Text('Amount: ${_order!.amount} CKB'),
          if (_error != null) ...[
            const SizedBox(height: 8),
            Text(_error!),
          ],
          const SizedBox(height: 16),
          const Text('Log'),
          const SizedBox(height: 8),
          if (_order == null || _order!.log.isEmpty)
            const Text('No log lines yet.')
          else
            for (final line in _order!.log)
              Text('${line.at}  ${line.text}', key: const Key('order-log')),
        ],
      ),
    );
  }
}
