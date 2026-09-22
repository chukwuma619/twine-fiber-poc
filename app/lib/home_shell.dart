import 'dart:async';

import 'package:flutter/material.dart';

import 'daemon_api.dart';
import 'fiber_api.dart';
import 'models.dart';
import 'offer_card.dart';
import 'post_ad_screen.dart';
import 'settings.dart';
import 'settings_screen.dart';
import 'take_ad_dialog.dart';
import 'trade_screen.dart';

enum BookSide { buy, sell }

enum HomeTab { book, trades }

class HomeShell extends StatefulWidget {
  const HomeShell({
    super.key,
    required this.settings,
    required this.daemon,
    required this.fiber,
  });

  final SettingsController settings;
  final DaemonApi daemon;
  final FiberApi fiber;

  @override
  State<HomeShell> createState() => _HomeShellState();
}

class _HomeShellState extends State<HomeShell> {
  List<AdSnapshot> _ads = const [];
  List<TradeSnapshot> _trades = const [];
  String? _error;
  var _loading = true;
  var _side = BookSide.buy;
  var _tab = HomeTab.book;
  Timer? _poll;

  UserSettings get _user => widget.settings.settings;

  List<TradeSnapshot> get _incomingOrders => [
    for (final trade in _trades)
      if (trade.isLister(_user.pubkey) && trade.isWaitingHold) trade,
  ];

  List<TradeSnapshot> get _openListedTrades => [
    for (final trade in _trades)
      if (trade.isLister(_user.pubkey) && trade.isOpen) trade,
  ];

  int get _actionCount => _trades.where((trade) {
    if (trade.isLister(_user.pubkey)) {
      return trade.isWaitingHold || trade.isFiatSent || trade.isReleasing;
    }
    if (trade.isTaker(_user.pubkey)) {
      return trade.isWaitingFiat || trade.isLeg2Failed;
    }
    return false;
  }).length;

  @override
  void initState() {
    super.initState();
    widget.settings.addListener(_onSettings);
    _load();
    _poll = Timer.periodic(const Duration(seconds: 5), (_) {
      _fetch(showSpinner: false);
    });
  }

  @override
  void dispose() {
    _poll?.cancel();
    widget.settings.removeListener(_onSettings);
    super.dispose();
  }

  void _onSettings() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        _load();
      }
    });
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
      final ads = await widget.daemon.listAds(_user.daemonUrl);
      final trades = await widget.daemon.listTrades(
        _user.daemonUrl,
        pubkey: _user.pubkey,
      );
      if (!mounted) {
        return;
      }
      setState(() {
        _ads = ads;
        _trades = trades;
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

  Future<void> _openSettings() async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => SettingsScreen(
          settings: widget.settings,
          daemon: widget.daemon,
          fiber: widget.fiber,
        ),
      ),
    );
    await _load();
  }

  Future<void> _openPostAd() async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => PostAdScreen(
          settings: widget.settings,
          daemon: widget.daemon,
          fiber: widget.fiber,
        ),
      ),
    );
    await _load();
  }

  Future<void> _openTrade(String id) async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TradeScreen(
          settings: widget.settings,
          daemon: widget.daemon,
          fiber: widget.fiber,
          tradeId: id,
        ),
      ),
    );
    await _load();
  }

  Future<void> _take(AdSnapshot ad) async {
    final pay = await showDialog<String>(
      context: context,
      builder: (context) => TakeAdDialog(ad: ad),
    );
    if (pay == null || !mounted) {
      return;
    }
    final pubkey = _user.pubkey;
    if (pubkey == null || pubkey.isEmpty) {
      setState(() {
        _error = 'Open Settings and read your Fiber node pubkey first.';
      });
      return;
    }
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final trade = await widget.daemon.createTrade(
        _user.daemonUrl,
        adId: ad.id,
        taker: pubkey,
        payAmount: pay,
      );
      if (!mounted) {
        return;
      }
      await _openTrade(trade.id);
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

  List<AdSnapshot> get _visibleAds {
    switch (_side) {
      case BookSide.buy:
        return [
          for (final ad in _ads)
            if (!ad.isMine(_user.pubkey)) ad,
        ];
      case BookSide.sell:
        return [
          for (final ad in _ads)
            if (ad.isMine(_user.pubkey)) ad,
        ];
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        leading: IconButton(
          key: const Key('open-settings'),
          onPressed: _openSettings,
          icon: const Icon(Icons.menu),
        ),
        title: Text(
          _user.labSeat == null
              ? 'Twine'
              : _user.labSeat == 'B'
              ? 'Twine · User B'
              : 'Twine · User A',
        ),
        actions: [
          IconButton(
            key: const Key('refresh-market'),
            onPressed: _loading ? null : _load,
            icon: const Icon(Icons.refresh),
          ),
        ],
      ),
      body: Column(
        children: [
          if (_tab == HomeTab.book) _BookTabs(
            side: _side,
            onChanged: (side) => setState(() => _side = side),
          ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
              child: Text(_error!, key: const Key('error-message')),
            ),
          Expanded(
            child: _tab == HomeTab.book ? _bookBody() : _tradesBody(),
          ),
        ],
      ),
      floatingActionButton: _tab == HomeTab.book
          ? FloatingActionButton(
              key: const Key('open-post-ad'),
              onPressed: _openPostAd,
              child: const Icon(Icons.add),
            )
          : null,
      bottomNavigationBar: NavigationBar(
        selectedIndex: _tab == HomeTab.book ? 0 : 1,
        onDestinationSelected: (index) {
          setState(() {
            _tab = index == 0 ? HomeTab.book : HomeTab.trades;
          });
        },
        destinations: [
          const NavigationDestination(
            icon: Icon(Icons.menu_book_outlined),
            selectedIcon: Icon(Icons.menu_book),
            label: 'Order book',
          ),
          NavigationDestination(
            icon: Badge(
              isLabelVisible: _actionCount > 0,
              label: Text('$_actionCount', key: const Key('my-trades-badge')),
              child: const Icon(Icons.swap_horiz_outlined),
            ),
            selectedIcon: Badge(
              isLabelVisible: _actionCount > 0,
              label: Text('$_actionCount'),
              child: const Icon(Icons.swap_horiz),
            ),
            label: 'My trades',
          ),
        ],
      ),
    );
  }

  Widget _bookBody() {
    final ads = _visibleAds;
    final incoming = _incomingOrders;
    if (_loading && _ads.isEmpty && _trades.isEmpty) {
      return const Center(child: Text('Loading'));
    }
    if (ads.isEmpty) {
      final hidden = _side == BookSide.sell && _openListedTrades.isNotEmpty
          ? _openListedTrades.first
          : null;
      return ListView(
        padding: const EdgeInsets.fromLTRB(16, 16, 16, 88),
        children: [
          if (incoming.isNotEmpty) ...[
            _incomingBanner(incoming.first),
            const SizedBox(height: 16),
          ],
          if (hidden != null) ...[
            Text(
              'A buyer already placed an order on your offer.',
              textAlign: TextAlign.center,
              style: TextStyle(
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: 16),
            FilledButton(
              key: const Key('open-hidden-trade'),
              onPressed: () => _openTrade(hidden.id),
              child: Text(hidden.statusFor(_user.pubkey)),
            ),
          ] else
            Padding(
              padding: const EdgeInsets.only(top: 48),
              child: Text(
                _side == BookSide.buy
                    ? 'No one is selling CKB yet.'
                    : 'You have no sell offers. Tap + to post one.',
                textAlign: TextAlign.center,
                style: TextStyle(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
              ),
            ),
        ],
      );
    }
    return ListView.separated(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 88),
      itemCount: ads.length + (incoming.isNotEmpty ? 1 : 0),
      separatorBuilder: (_, _) => const SizedBox(height: 12),
      itemBuilder: (context, index) {
        if (incoming.isNotEmpty && index == 0) {
          return _incomingBanner(incoming.first);
        }
        final ad = ads[incoming.isNotEmpty ? index - 1 : index];
        final mine = ad.isMine(_user.pubkey);
        return OfferCard(
          ad: ad,
          mine: mine,
          onTake: mine || _loading ? null : () => _take(ad),
        );
      },
    );
  }

  Widget _incomingBanner(TradeSnapshot trade) {
    return Card(
      key: const Key('new-order-banner'),
      child: ListTile(
        title: const Text('New order — accept'),
        subtitle: Text(
          '${trade.payAmount} ${trade.currency} · ${trade.amount} CKB',
        ),
        trailing: const Icon(Icons.chevron_right),
        onTap: () => _openTrade(trade.id),
      ),
    );
  }

  Widget _tradesBody() {
    if (_loading && _trades.isEmpty) {
      return const Center(child: Text('Loading'));
    }
    if (_trades.isEmpty) {
      return Center(
        child: Text(
          'No trades yet.',
          style: TextStyle(color: Theme.of(context).colorScheme.onSurfaceVariant),
        ),
      );
    }
    final ordered = [..._trades]
      ..sort((left, right) {
        final leftOpen = left.watchesHoldExpiry ? 0 : 1;
        final rightOpen = right.watchesHoldExpiry ? 0 : 1;
        return leftOpen.compareTo(rightOpen);
      });
    return ListView.separated(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 24),
      itemCount: ordered.length,
      separatorBuilder: (_, _) => const SizedBox(height: 12),
      itemBuilder: (context, index) {
        final trade = ordered[index];
        return Card(
          key: Key('my-trade-${trade.id}'),
          child: ListTile(
            title: Text('${trade.amount} CKB · ${trade.statusFor(_user.pubkey)}'),
            subtitle: Text(
              '${trade.payAmount} ${trade.currency}',
            ),
            trailing: const Icon(Icons.chevron_right),
            onTap: () => _openTrade(trade.id),
          ),
        );
      },
    );
  }
}

class _BookTabs extends StatelessWidget {
  const _BookTabs({required this.side, required this.onChanged});

  final BookSide side;
  final ValueChanged<BookSide> onChanged;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 0, 8, 4),
      child: Row(
        children: [
          _tab(context, BookSide.buy, 'BUY CKB'),
          _tab(context, BookSide.sell, 'SELL CKB'),
        ],
      ),
    );
  }

  Widget _tab(BuildContext context, BookSide value, String label) {
    final selected = side == value;
    final colors = Theme.of(context).colorScheme;
    return Expanded(
      child: InkWell(
        key: Key(value == BookSide.buy ? 'buy-tab' : 'sell-tab'),
        onTap: () => onChanged(value),
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 12),
              child: Text(
                label,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontWeight: FontWeight.w600,
                  letterSpacing: 0.4,
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
