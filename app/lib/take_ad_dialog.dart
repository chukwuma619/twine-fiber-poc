import 'package:flutter/material.dart';

import 'amounts.dart';
import 'models.dart';

class TakeAdDialog extends StatefulWidget {
  const TakeAdDialog({super.key, required this.ad});

  final AdSnapshot ad;

  @override
  State<TakeAdDialog> createState() => _TakeAdDialogState();
}

class _TakeAdDialogState extends State<TakeAdDialog> {
  final TextEditingController _pay = TextEditingController();
  String? _ckb;
  String? _error;

  @override
  void dispose() {
    _pay.dispose();
    super.dispose();
  }

  void _recompute() {
    final ad = widget.ad;
    final pay = _pay.text.trim();
    try {
      if (pay.isEmpty) {
        setState(() {
          _ckb = null;
          _error = null;
        });
        return;
      }
      if (ad.min.isNotEmpty && compareAmount(pay, ad.min) < 0) {
        setState(() {
          _ckb = null;
          _error = 'Minimum is ${ad.min} ${ad.currency}';
        });
        return;
      }
      final cap = ad.max.isEmpty
          ? payFromCkb(ad.available, ad.price)
          : takeCap(ad.max, ad.available, ad.price);
      if (compareAmount(pay, cap) > 0) {
        setState(() {
          _ckb = null;
          _error = 'Maximum is $cap ${ad.currency}';
        });
        return;
      }
      final ckb = ckbFromPay(pay, ad.price);
      setState(() {
        _ckb = ckb;
        _error = null;
      });
    } catch (err) {
      setState(() {
        _ckb = null;
        _error = err.toString();
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final ad = widget.ad;
    return AlertDialog(
      title: const Text('Take offer'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Currency ${ad.currency}'),
          Text('Price ${ad.price} per CKB'),
          Text('Available ${ad.available} CKB'),
          if (ad.min.isNotEmpty && ad.max.isNotEmpty)
            Text('Limit ${ad.min}–${ad.max}'),
          TextField(
            key: const Key('pay-amount'),
            controller: _pay,
            decoration: const InputDecoration(labelText: 'You pay'),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
            onChanged: (_) => _recompute(),
          ),
          if (_ckb != null)
            Text('Locks $_ckb CKB', key: const Key('ckb-preview')),
          if (_error != null) Text(_error!),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('start-trade'),
          onPressed: _ckb == null
              ? null
              : () => Navigator.of(context).pop(_pay.text.trim()),
          child: const Text('Start trade'),
        ),
      ],
    );
  }
}
