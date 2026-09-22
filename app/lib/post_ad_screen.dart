import 'package:flutter/material.dart';

import 'daemon_api.dart';
import 'fiber_api.dart';
import 'settings.dart';

class PostAdScreen extends StatefulWidget {
  const PostAdScreen({
    super.key,
    required this.settings,
    required this.daemon,
    required this.fiber,
  });

  final SettingsController settings;
  final DaemonApi daemon;
  final FiberApi fiber;

  @override
  State<PostAdScreen> createState() => _PostAdScreenState();
}

class _PostAdScreenState extends State<PostAdScreen> {
  final TextEditingController _amount = TextEditingController();
  final TextEditingController _currency = TextEditingController(text: 'NGN');
  final TextEditingController _price = TextEditingController();
  final TextEditingController _min = TextEditingController();
  final TextEditingController _max = TextEditingController();
  final TextEditingController _method = TextEditingController(text: 'Opay');
  String? _error;
  var _busy = false;

  UserSettings get _user => widget.settings.settings;

  String get _currencyCode {
    final text = _currency.text.trim();
    return text.isEmpty ? 'NGN' : text.toUpperCase();
  }

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      var pubkey = _user.pubkey;
      if (pubkey == null || pubkey.isEmpty) {
        pubkey = await widget.fiber.nodePubkey(_user.fiberRpc);
        await widget.settings.update(_user.copyWith(pubkey: pubkey));
      }
      await widget.daemon.createAd(
        _user.daemonUrl,
        pubkey: pubkey,
        available: _amount.text,
        currency: _currencyCode,
        price: _price.text,
        min: _min.text,
        max: _max.text,
        paymentMethod: _method.text,
      );
      if (!mounted) {
        return;
      }
      Navigator.of(context).pop();
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

  @override
  void dispose() {
    _amount.dispose();
    _currency.dispose();
    _price.dispose();
    _min.dispose();
    _max.dispose();
    _method.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('New sell offer')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          TextField(
            key: const Key('ad-amount'),
            controller: _amount,
            decoration: const InputDecoration(labelText: 'Available'),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
          ),
          TextField(
            key: const Key('ad-currency'),
            controller: _currency,
            decoration: const InputDecoration(
              labelText: 'Currency',
              hintText: 'NGN, GHS, USD…',
            ),
            textCapitalization: TextCapitalization.characters,
          ),
          TextField(
            key: const Key('ad-price'),
            controller: _price,
            decoration: const InputDecoration(
              labelText: 'Price',
              hintText: 'for 1 CKB',
            ),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
          ),
          TextField(
            key: const Key('ad-min'),
            controller: _min,
            decoration: const InputDecoration(labelText: 'Min per trade'),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
          ),
          TextField(
            key: const Key('ad-max'),
            controller: _max,
            decoration: const InputDecoration(labelText: 'Max per trade'),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
          ),
          TextField(
            key: const Key('ad-method'),
            controller: _method,
            decoration: const InputDecoration(
              labelText: 'Payment (rail and handle)',
            ),
          ),
          const SizedBox(height: 16),
          FilledButton(
            key: const Key('post-ad'),
            onPressed: _busy ? null : _submit,
            child: Text(_busy ? 'Working...' : 'Post ad'),
          ),
          if (_error != null) ...[
            const SizedBox(height: 12),
            Text(_error!, key: const Key('error-message')),
          ],
        ],
      ),
    );
  }
}
