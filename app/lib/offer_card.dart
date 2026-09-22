import 'package:flutter/material.dart';

import 'models.dart';

class OfferCard extends StatelessWidget {
  const OfferCard({
    super.key,
    required this.ad,
    required this.mine,
    this.onTake,
  });

  final AdSnapshot ad;
  final bool mine;
  final VoidCallback? onTake;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final muted = theme.colorScheme.onSurfaceVariant;
    final currency = ad.currency.isEmpty ? 'NGN' : ad.currency;

    return Card(
      key: Key('ad-${ad.id}'),
      child: InkWell(
        key: Key('take-${ad.id}'),
        onTap: onTake,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 14, 16, 16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (mine) ...[
                Text(
                  'YOUR OFFER',
                  style: theme.textTheme.labelMedium?.copyWith(color: muted),
                ),
                const SizedBox(height: 10),
              ],
              Row(
                crossAxisAlignment: CrossAxisAlignment.baseline,
                textBaseline: TextBaseline.alphabetic,
                children: [
                  Text(
                    ad.price,
                    style: theme.textTheme.headlineMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                      letterSpacing: -0.6,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text(
                    '$currency/CKB',
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              _Fact(
                label: 'Available',
                value: '${ad.available} CKB',
                color: muted,
              ),
              if (ad.min.isNotEmpty && ad.max.isNotEmpty) ...[
                const SizedBox(height: 4),
                _Fact(
                  label: 'Limit',
                  value: '${ad.min}–${ad.max} $currency',
                  color: muted,
                ),
              ],
              const SizedBox(height: 14),
              Row(
                children: [
                  Icon(
                    Icons.account_balance_wallet_outlined,
                    size: 16,
                    color: muted,
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      ad.paymentMethod,
                      style: theme.textTheme.bodyMedium,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Fact extends StatelessWidget {
  const _Fact({
    required this.label,
    required this.value,
    required this.color,
  });

  final String label;
  final String value;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final style = Theme.of(context).textTheme.bodyMedium?.copyWith(color: color);
    return Row(
      children: [
        SizedBox(width: 88, child: Text(label, style: style)),
        Expanded(child: Text(value, style: style)),
      ],
    );
  }
}
